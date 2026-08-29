//! Retention behaviour that the generated contract properties do not reach:
//! how truncation interacts with sequence identity, with addressing below the
//! horizon, and with forking.

use ironstate::prelude::*;
use ironstate_aggregate::{
    Aggregate, AggregateRules, DrawPos, LogicalTime, OwnedDeterministicCtx, Seed, SeededEntropy,
    StableHash,
};
use ironstate_journal::{
    ForkableJournal, Journal, JournalError, MemoryJournal, RetainableJournal, Seq, Snapshot,
    StreamId, execute, resume,
};

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
    StreamId::new("retention")
}

fn ctx(seed: &Seed, pos: DrawPos) -> OwnedDeterministicCtx<u32> {
    OwnedDeterministicCtx {
        entropy: Box::new(SeededEntropy::at(seed, pos)),
        actor: 0,
        now: LogicalTime(0),
    }
}

/// Drive `steps` appends, then snapshot at the head so truncation is legal.
fn driven(steps: u64) -> (MemoryJournal<Counter>, Aggregate<Counter>, Seed) {
    let seed = Seed([3; 32]);
    let mut journal = MemoryJournal::new(genesis());
    let mut agg = Aggregate::new(genesis()).unwrap();
    for _ in 0..steps {
        let pos = journal
            .head(&stream())
            .map_or(DrawPos(0), |h| journal.entropy_pos(&stream(), h).unwrap());
        let mut c = ctx(&seed, pos);
        execute(&mut journal, &stream(), &mut agg, &Command::Tick, &mut c).unwrap();
    }
    let head = journal.head(&stream()).unwrap();
    let pos = journal.entropy_pos(&stream(), head).unwrap();
    journal
        .snapshot_in(
            &mut (),
            &stream(),
            Snapshot {
                state: agg.state().clone(),
                schema_version: 0,
                at: head,
                entropy_pos: pos,
            },
        )
        .unwrap();
    (journal, agg, seed)
}

#[test]
fn retained_records_keep_their_sequence_numbers() {
    let (mut journal, _agg, _seed) = driven(6);
    let before = journal.entropy_pos(&stream(), Seq(5)).unwrap();

    journal.truncate_before(&stream(), Seq(4)).unwrap();

    assert_eq!(journal.retained_from(&stream()), Seq(4));
    assert_eq!(
        journal.head(&stream()),
        Some(Seq(6)),
        "truncation must not renumber the head",
    );
    assert_eq!(
        journal.entropy_pos(&stream(), Seq(5)).unwrap(),
        before,
        "a retained record must keep its Seq and its recorded position",
    );
}

#[test]
fn addressing_below_the_horizon_is_unknown_not_a_neighbour() {
    let (mut journal, _agg, _seed) = driven(6);
    journal.truncate_before(&stream(), Seq(4)).unwrap();

    for gone in [Seq(1), Seq(2), Seq(3)] {
        match journal.entropy_pos(&stream(), gone) {
            Err(JournalError::UnknownSeq { at }) => assert_eq!(at, gone),
            other => panic!("expected UnknownSeq for truncated {gone:?}, got {other:?}"),
        }
    }
}

#[test]
fn truncation_is_refused_when_no_snapshot_covers_the_retained_prefix() {
    let seed = Seed([3; 32]);
    let mut journal = MemoryJournal::new(genesis());
    let mut agg = Aggregate::new(genesis()).unwrap();
    for _ in 0..4 {
        let pos = journal
            .head(&stream())
            .map_or(DrawPos(0), |h| journal.entropy_pos(&stream(), h).unwrap());
        let mut c = ctx(&seed, pos);
        execute(&mut journal, &stream(), &mut agg, &Command::Tick, &mut c).unwrap();
    }
    // The only snapshot is the genesis at Seq(0); truncating before Seq(3)
    // would strand records 1..2 that replay from it still needs.
    match journal.truncate_before(&stream(), Seq(3)) {
        Err(JournalError::NoSnapshotForTruncation {
            at,
            latest_snapshot,
        }) => {
            assert_eq!(at, Seq(3));
            assert_eq!(latest_snapshot, Some(Seq(0)));
        }
        other => panic!("expected refusal, got {other:?}"),
    }
    assert_eq!(
        journal.head(&stream()),
        Some(Seq(4)),
        "a refused truncation must change nothing",
    );
}

#[test]
fn a_truncated_stream_still_resumes() {
    let (mut journal, agg, seed) = driven(6);
    let before = agg.state().clone();
    journal.truncate_before(&stream(), Seq(4)).unwrap();
    let (resumed, _) = resume::<Counter, _>(&journal, &stream(), &seed).unwrap();
    assert_eq!(resumed.state(), &before);
}

#[test]
fn forking_below_the_retained_horizon_is_refused() {
    let (mut journal, _agg, _seed) = driven(6);
    journal.truncate_before(&stream(), Seq(4)).unwrap();

    // Records 1..3 are gone, so no fork can reproduce history through them.
    for gone in [Seq(1), Seq(2), Seq(3)] {
        assert!(
            matches!(
                journal.fork(&stream(), gone),
                Err(JournalError::UnknownSeq { .. })
            ),
            "forking at {gone:?}, below the horizon, must be refused",
        );
    }
}

#[test]
fn a_fork_of_a_truncated_stream_keeps_the_fork_point_as_its_head() {
    let (mut journal, _agg, _seed) = driven(6);
    journal.truncate_before(&stream(), Seq(4)).unwrap();

    let branch = journal.fork(&stream(), Seq(5)).expect("fork above horizon");
    assert_eq!(
        branch.head(&stream()),
        Some(Seq(5)),
        "a fork's head must sit at the fork point, truncated or not",
    );
    assert_eq!(
        branch.entropy_pos(&stream(), Seq(5)).unwrap(),
        journal.entropy_pos(&stream(), Seq(5)).unwrap(),
        "a fork must agree with its source at the fork point",
    );
    assert_eq!(branch.retained_from(&stream()), Seq(4));
}

#[test]
fn a_fork_of_a_truncated_stream_still_has_a_replay_base() {
    let (mut journal, _agg, seed) = driven(6);
    journal.truncate_before(&stream(), Seq(4)).unwrap();

    let branch = journal.fork(&stream(), Seq(6)).expect("fork at head");
    resume::<Counter, _>(&branch, &stream(), &seed)
        .expect("a fork must carry a snapshot to replay from");
}

/// A snapshot taken *below* the truncation horizon is no longer a valid replay
/// base: replaying from it needs records that are gone. Truncation must drop
/// such snapshots, so a fork that lands between them fails honestly rather than
/// silently replaying to a wrong state.
///
/// Regression: before this was fixed, the fork below resumed to a state built
/// from the genesis snapshot plus only the *retained* records — a wrong
/// aggregate returned as `Ok`.
#[test]
fn a_stale_base_below_the_horizon_never_produces_a_wrong_state() {
    // The true state at the fork point, captured before anything is truncated.
    let (untruncated, _agg, seed) = driven(6);
    let branch = untruncated
        .fork(&stream(), Seq(5))
        .expect("fork of an untruncated stream");
    let truth = resume::<Counter, _>(&branch, &stream(), &seed)
        .expect("resume")
        .0
        .state()
        .clone();

    let (mut journal, _agg, _seed) = driven(6);
    journal.truncate_before(&stream(), Seq(4)).unwrap();

    match journal.fork(&stream(), Seq(5)) {
        Err(_) => {}
        Ok(branch) => match resume::<Counter, _>(&branch, &stream(), &seed) {
            // Honest: no surviving snapshot covers this fork point.
            Err(_) => {}
            Ok((resumed, _)) => assert_eq!(
                resumed.state(),
                &truth,
                "a fork must never replay from a base whose records were truncated away",
            ),
        },
    }
}
