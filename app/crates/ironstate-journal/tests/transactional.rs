//! The transactional seam: a journal that enlists in the **caller's** unit of
//! work, and the atomicity property that seam has to satisfy.
//!
//! `MemoryJournal` owns its own durability (`Tx<'_> = ()`), so it cannot
//! exercise this. The `StagedJournal` below is the smallest thing that can: a
//! journal whose transaction is a staging buffer, where appends become visible
//! only when the caller commits. That is the shape a relational adapter has,
//! reduced to the part the contract cares about.

use ironstate::prelude::*;
use ironstate_aggregate::{
    Aggregate, AggregateRules, DrawPos, LogicalTime, OwnedDeterministicCtx, Seed, SeededEntropy,
    StableHash,
};
use ironstate_journal::{
    Journal, JournalError, Seq, Snapshot, StreamId, VersionedEvent, execute_in,
};
use std::borrow::Cow;
use std::collections::BTreeMap;

// === the aggregate under test =============================================

#[derive(StateMachine, StableHash, Clone, Debug, PartialEq)]
#[state_machine(initial = Open, terminal = [Closed])]
enum Phase {
    Open,
    Closed,
}
#[derive(Event, Clone, Debug, PartialEq)]
enum Step {
    Close,
}
impl TransitionRules for Phase {
    type Event = Step;
    fn transition(&self, _: &Step) -> Option<Phase> {
        matches!(self, Phase::Open).then_some(Phase::Closed)
    }
}

#[derive(Event, Clone, Debug, PartialEq)]
enum Command {
    Tick,
}
#[derive(Clone, Debug, PartialEq)]
enum Ev {
    Rolled(u8),
}
#[derive(Debug, thiserror::Error)]
#[error("closed")]
struct ClosedErr;

#[derive(StableHash, Clone, Debug, PartialEq)]
struct Counter {
    phase: Phase,
    total: u32,
}

impl AggregateRules for Counter {
    type Phase = Phase;
    type Command = Command;
    type Event = Ev;
    type Error = ClosedErr;
    type Ctx = OwnedDeterministicCtx<u32>;

    fn phase(&self) -> Phase {
        self.phase.clone()
    }
    fn decide(&self, _cmd: &Command, ctx: &mut Self::Ctx) -> Result<Vec<Ev>, ClosedErr> {
        if self.phase != Phase::Open {
            return Err(ClosedErr);
        }
        Ok(vec![Ev::Rolled(ctx.entropy.draw_range(1..7) as u8)])
    }
    fn evolve(&mut self, event: &Ev) {
        let Ev::Rolled(n) = event;
        self.total += u32::from(*n);
    }
}

fn genesis() -> Counter {
    Counter {
        phase: Phase::Open,
        total: 0,
    }
}

fn stream() -> StreamId {
    StreamId::new("staged")
}

// === a journal with a real unit of work ===================================

#[derive(Clone)]
struct Record {
    events: Vec<Ev>,
    entropy_pos: DrawPos,
}

/// The caller's unit of work: appends land here first and reach the journal
/// only if this is handed back to [`StagedJournal::commit`]. Dropping it
/// instead is the rollback.
#[derive(Default)]
struct Staged {
    pending: Vec<(StreamId, Record)>,
}

/// A journal whose appends enlist in a caller-supplied [`Staged`] transaction.
struct StagedJournal {
    committed: BTreeMap<StreamId, Vec<Record>>,
    genesis: Counter,
}

impl StagedJournal {
    fn new(genesis: Counter) -> Self {
        Self {
            committed: BTreeMap::new(),
            genesis,
        }
    }

    /// Apply everything the transaction staged. Anything not committed this way
    /// never becomes visible.
    fn commit(&mut self, tx: Staged) {
        for (stream, record) in tx.pending {
            self.committed.entry(stream).or_default().push(record);
        }
    }

    fn records(&self, stream: &StreamId) -> &[Record] {
        self.committed.get(stream).map_or(&[], Vec::as_slice)
    }
}

impl Journal<Counter> for StagedJournal {
    type Tx<'a> = Staged;

    fn append_in(
        &mut self,
        tx: &mut Self::Tx<'_>,
        stream: &StreamId,
        events: &[Ev],
        entropy_pos: DrawPos,
    ) -> Result<Seq, JournalError> {
        tx.pending.push((
            stream.clone(),
            Record {
                events: events.to_vec(),
                entropy_pos,
            },
        ));
        // The sequence this append *will* occupy once the transaction commits:
        // what is already durable, plus what this transaction has staged for
        // the same stream.
        let staged_here = tx.pending.iter().filter(|(s, _)| s == stream).count();
        Ok(Seq((self.records(stream).len() + staged_here) as u64))
    }

    fn snapshot_in(
        &mut self,
        _tx: &mut Self::Tx<'_>,
        _stream: &StreamId,
        _snapshot: Snapshot<Counter>,
    ) -> Result<(), JournalError> {
        Ok(())
    }

    fn entropy_pos(&self, stream: &StreamId, at: Seq) -> Result<DrawPos, JournalError> {
        if at.0 == 0 {
            return Ok(DrawPos(0));
        }
        let records = self.records(stream);
        if at.0 > records.len() as u64 {
            return Err(JournalError::UnknownSeq { at });
        }
        Ok(records[(at.0 - 1) as usize].entropy_pos)
    }

    fn head(&self, stream: &StreamId) -> Option<Seq> {
        let records = self.records(stream);
        (!records.is_empty()).then_some(Seq(records.len() as u64))
    }

    fn events_since(
        &self,
        stream: &StreamId,
        after: Option<Seq>,
    ) -> Result<Vec<VersionedEvent<Counter>>, JournalError> {
        let start = after.map_or(0, |s| usize::try_from(s.0).unwrap_or(usize::MAX));
        let type_name = Cow::Borrowed(core::any::type_name::<Ev>());
        Ok(self
            .records(stream)
            .iter()
            .enumerate()
            .skip(start)
            .flat_map(|(i, r)| {
                let seq = Seq(i as u64 + 1);
                r.events.iter().map(move |event| (seq, event))
            })
            .map(|(seq, event)| VersionedEvent {
                event: event.clone(),
                seq,
                type_name: type_name.clone(),
                version: 1,
            })
            .collect())
    }

    fn latest_snapshot(
        &self,
        _stream: &StreamId,
    ) -> Result<Option<Snapshot<Counter>>, JournalError> {
        Ok(Some(Snapshot {
            state: self.genesis.clone(),
            schema_version: 0,
            at: Seq(0),
            entropy_pos: DrawPos(0),
        }))
    }
}

fn ctx(seed: &Seed, pos: DrawPos) -> OwnedDeterministicCtx<u32> {
    OwnedDeterministicCtx {
        entropy: Box::new(SeededEntropy::at(seed, pos)),
        actor: 0,
        now: LogicalTime(0),
    }
}

// === the properties =======================================================

/// An append inside a unit of work that is subsequently rolled back leaves the
/// journal at its prior head, the entropy stream rewound, and the aggregate
/// un-evolved.
///
/// The aggregate half is the part a journal-only check would miss: `execute_in`
/// deliberately does not evolve, precisely so a rollback cannot strand the
/// in-memory state ahead of the durable log.
#[test]
fn rollback_leaves_nothing_observable() {
    let seed = Seed([4; 32]);
    let mut journal = StagedJournal::new(genesis());
    // Deliberately not `mut`: a rolled-back append must never need to touch it.
    let agg = Aggregate::new(genesis()).unwrap();
    let before_state = agg.state().clone();
    let before_head = journal.head(&stream());

    let mut tx = Staged::default();
    let mut context = ctx(&seed, DrawPos(0));
    let pending = execute_in(
        &mut journal,
        &mut tx,
        &stream(),
        &agg,
        &Command::Tick,
        &mut context,
    )
    .expect("the append reaches the staging buffer");

    // The transaction is rolled back — dropped rather than committed.
    drop(tx);
    pending.abort(&mut context);

    assert_eq!(
        journal.head(&stream()),
        before_head,
        "a rolled-back append must leave the head where it was",
    );
    assert_eq!(
        journal.events_since(&stream(), None).unwrap().len(),
        0,
        "a rolled-back append must leave no events behind",
    );
    assert_eq!(
        agg.state(),
        &before_state,
        "a rolled-back append must not have evolved the aggregate",
    );
    assert_eq!(
        context.entropy.draws(),
        DrawPos(0),
        "a rolled-back append must leave the entropy stream rewound",
    );
}

/// The mirror image: when the caller's transaction commits, the append becomes
/// visible and `Pending::commit` brings the aggregate up to the log.
#[test]
fn commit_makes_the_append_visible_and_advances_the_aggregate() {
    let seed = Seed([4; 32]);
    let mut journal = StagedJournal::new(genesis());
    let mut agg = Aggregate::new(genesis()).unwrap();

    let mut tx = Staged::default();
    let mut context = ctx(&seed, DrawPos(0));
    let pending = execute_in(
        &mut journal,
        &mut tx,
        &stream(),
        &agg,
        &Command::Tick,
        &mut context,
    )
    .expect("append staged");

    // Not visible until the caller's transaction commits.
    assert_eq!(
        journal.head(&stream()),
        None,
        "a staged append must not be visible before the transaction commits",
    );

    journal.commit(tx);
    let seq = pending.commit(&mut agg);

    assert_eq!(seq, Seq(1));
    assert_eq!(journal.head(&stream()), Some(Seq(1)));
    assert_eq!(
        journal.events_since(&stream(), None).unwrap().len(),
        1,
        "the committed append must be visible",
    );
    assert!(
        agg.state().total > 0,
        "committing must advance the aggregate to match the log",
    );
}

/// The caller's other writes and the append share one transaction: either all
/// of it lands or none of it does. This is the transactional-outbox guarantee
/// the seam exists to provide.
#[test]
fn the_append_and_the_callers_writes_commit_together() {
    let seed = Seed([7; 32]);
    let mut journal = StagedJournal::new(genesis());
    let mut agg = Aggregate::new(genesis()).unwrap();

    // Stand-in for a read-model row and an outbound job. Like the journal's own
    // records these are staged first and only applied when the transaction
    // commits, so a rollback must leave `durable` empty.
    let mut durable: Vec<&str> = Vec::new();

    let mut tx = Staged::default();
    let mut staged: Vec<&str> = Vec::new();
    let mut context = ctx(&seed, DrawPos(0));
    let pending = execute_in(
        &mut journal,
        &mut tx,
        &stream(),
        &agg,
        &Command::Tick,
        &mut context,
    )
    .expect("append staged");
    staged.push("read-model row");
    staged.push("outbound notice");

    // The transaction fails after both the append and the caller's own writes
    // were staged; neither may survive.
    drop(tx);
    staged.clear();
    pending.abort(&mut context);

    assert_eq!(journal.head(&stream()), None);
    assert!(
        durable.is_empty(),
        "the caller's writes roll back with the append",
    );

    // Now the same sequence, committed.
    let mut tx = Staged::default();
    let mut staged: Vec<&str> = Vec::new();
    let mut context = ctx(&seed, DrawPos(0));
    let pending = execute_in(
        &mut journal,
        &mut tx,
        &stream(),
        &agg,
        &Command::Tick,
        &mut context,
    )
    .expect("append staged");
    staged.push("read-model row");
    staged.push("outbound notice");

    journal.commit(tx);
    durable.append(&mut staged);
    pending.commit(&mut agg);

    assert_eq!(journal.head(&stream()), Some(Seq(1)));
    assert_eq!(
        durable,
        ["read-model row", "outbound notice"],
        "the caller's writes land with the append",
    );
}
