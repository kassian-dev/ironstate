//! Retention behaviour that the generated contract properties do not reach:
//! how truncation interacts with sequence identity, with addressing below the
//! horizon, and with forking.

use ironstate::prelude::*;
use ironstate_aggregate::{
    Aggregate, AggregateRules, DrawPos, EntropySource, LogicalTime, OwnedDeterministicCtx, Seed,
    SeededEntropy, StableHash,
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

    // Seq(6) carries the surviving snapshot, so it is a forkable point.
    let branch = journal.fork(&stream(), Seq(6)).expect("fork above horizon");
    assert_eq!(
        branch.head(&stream()),
        Some(Seq(6)),
        "a fork's head must sit at the fork point, truncated or not",
    );
    assert_eq!(
        branch.entropy_pos(&stream(), Seq(6)).unwrap(),
        journal.entropy_pos(&stream(), Seq(6)).unwrap(),
        "a fork must agree with its source at the fork point",
    );
    assert_eq!(branch.retained_from(&stream()), Seq(4));
}

/// A fork point with no surviving snapshot at or below it is refused outright,
/// rather than handed back as a branch whose `resume` fails later.
#[test]
fn forking_where_no_base_survives_is_refused() {
    let (mut journal, _agg, _seed) = driven(6);
    journal.truncate_before(&stream(), Seq(4)).unwrap();

    // Truncation dropped the genesis snapshot; the only one left is at Seq(6).
    match journal.fork(&stream(), Seq(5)) {
        Err(JournalError::NoBaseForFork { at }) => assert_eq!(at, Seq(5)),
        Err(other) => panic!("expected NoBaseForFork, got {other:?}"),
        Ok(_) => panic!("expected NoBaseForFork, got a branch with no replay base"),
    }
}

/// A stream truncated all the way to its head still has a head. Reporting
/// `None` would tell `execute` the stream is empty, rewinding the live entropy
/// stream to `DrawPos(0)` — so positions would run backwards while sequences
/// ran forwards, breaking the determinism contract.
#[test]
fn a_fully_truncated_stream_still_reports_its_head() {
    let (mut journal, _agg, seed) = driven(6);
    let head_pos = journal.entropy_pos(&stream(), Seq(6)).unwrap();

    journal.truncate_before(&stream(), Seq(7)).unwrap();

    assert_eq!(journal.retained_from(&stream()), Seq(7));
    assert_eq!(
        journal.head(&stream()),
        Some(Seq(6)),
        "the records are gone, the history is not",
    );

    // The next append must continue the entropy stream, not restart it.
    let (mut resumed, entropy) = resume::<Counter, _>(&journal, &stream(), &seed).unwrap();
    assert_eq!(entropy.draws(), head_pos, "resume must not rewind entropy");
    let mut c = ctx(&seed, entropy.draws());
    execute(
        &mut journal,
        &stream(),
        &mut resumed,
        &Command::Tick,
        &mut c,
    )
    .unwrap();
    assert!(
        journal.entropy_pos(&stream(), Seq(7)).unwrap() > head_pos,
        "positions must keep moving forward across a full truncation",
    );
}

/// Reading from below the horizon must refuse rather than return a list with a
/// silent hole in it — a subscription resuming from a stale high-water mark
/// would otherwise drop records and corrupt its projection with no error.
#[test]
fn reading_from_below_the_horizon_is_refused_not_gapped() {
    let (mut journal, _agg, _seed) = driven(6);
    let before = journal.events_since(&stream(), Some(Seq(2))).unwrap().len();
    assert_eq!(before, 4);

    journal.truncate_before(&stream(), Seq(4)).unwrap();

    match journal.events_since(&stream(), Some(Seq(2))) {
        Err(JournalError::UnknownSeq { at }) => assert_eq!(at, Seq(2)),
        other => panic!(
            "expected UnknownSeq for a stale mark, got {:?}",
            other.map(|e| e.len())
        ),
    }
    // `None` is `after = Seq(0)` — genesis — not "whatever this stream still
    // has", so after truncation it is itself below the horizon and refused.
    match journal.events_since(&stream(), None) {
        Err(JournalError::UnknownSeq { at }) => assert_eq!(at, Seq(0)),
        other => panic!(
            "expected UnknownSeq from genesis, got {:?}",
            other.map(|e| e.len())
        ),
    }
    // Reading from the horizon itself is the valid way to say "everything you have".
    assert_eq!(
        journal.events_since(&stream(), Some(Seq(3))).unwrap().len(),
        3,
    );
}

/// Genesis is below the horizon once anything is truncated, so it must be as
/// unknown as any other discarded sequence.
#[test]
fn genesis_is_unknown_once_it_is_below_the_horizon() {
    let (mut journal, _agg, _seed) = driven(6);
    journal.truncate_before(&stream(), Seq(4)).unwrap();

    match journal.entropy_pos(&stream(), Seq(0)) {
        Err(JournalError::UnknownSeq { at }) => assert_eq!(at, Seq(0)),
        other => panic!("expected UnknownSeq for genesis below the horizon, got {other:?}"),
    }
}

/// A refused truncation must leave the journal byte-for-byte as it was —
/// including not bringing a stream into existence.
#[test]
fn a_refused_truncation_does_not_materialise_the_stream() {
    let (mut journal, _agg, _seed) = driven(6);
    let mut before = journal.streams().unwrap();
    before.sort();

    let typo = StreamId::new("typo");
    assert!(journal.truncate_before(&typo, Seq(5)).is_err());

    let mut after = journal.streams().unwrap();
    after.sort();
    assert_eq!(
        after, before,
        "a refused truncation must not add a phantom stream",
    );
}

/// A snapshot recorded beyond the head must not authorise an arbitrary
/// truncation — `retained_from` would then disagree with the `at` requested.
#[test]
fn truncation_past_the_head_is_refused() {
    let (mut journal, agg, _seed) = driven(6);
    journal
        .snapshot_in(
            &mut (),
            &stream(),
            Snapshot {
                state: agg.state().clone(),
                schema_version: 0,
                at: Seq(1000),
                entropy_pos: DrawPos(0),
            },
        )
        .unwrap();

    assert!(
        journal.truncate_before(&stream(), Seq(500)).is_err(),
        "a snapshot past the head must not authorise truncating past the head",
    );
    assert_eq!(journal.head(&stream()), Some(Seq(6)));
    assert_eq!(journal.retained_from(&stream()), Seq(1));

    // One past the head is the legitimate "discard everything" call.
    journal.truncate_before(&stream(), Seq(7)).unwrap();
    assert_eq!(journal.retained_from(&stream()), Seq(7));
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

/// `at` names the first record to keep, so `Seq(0)` — genesis, not a record —
/// is not a truncation point. Accepting it would report success while
/// `retained_from` still said `Seq(1)`.
#[test]
fn truncating_before_genesis_is_refused() {
    let (mut journal, _agg, _seed) = driven(6);
    match journal.truncate_before(&stream(), Seq(0)) {
        Err(JournalError::UnknownSeq { at }) => assert_eq!(at, Seq(0)),
        other => panic!("expected UnknownSeq for Seq(0), got {other:?}"),
    }
    assert_eq!(journal.retained_from(&stream()), Seq(1));
}

/// A snapshot recorded past the head describes state the stream does not have,
/// so it must not vouch for a truncation that would otherwise be illegal.
#[test]
fn a_snapshot_past_the_head_cannot_authorise_truncation() {
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
    // The only real snapshot is genesis at Seq(0). Add a bogus one past the head.
    journal
        .snapshot_in(
            &mut (),
            &stream(),
            Snapshot {
                state: agg.state().clone(),
                schema_version: 0,
                at: Seq(99),
                entropy_pos: DrawPos(0),
            },
        )
        .unwrap();

    // Truncating before Seq(3) needs a base at Seq(2) or later; only the bogus
    // snapshot qualifies, and it must not count.
    match journal.truncate_before(&stream(), Seq(3)) {
        Err(JournalError::NoSnapshotForTruncation {
            latest_snapshot, ..
        }) => assert_eq!(
            latest_snapshot,
            Some(Seq(0)),
            "the out-of-range snapshot must be ignored when reporting the newest base",
        ),
        other => panic!("expected refusal, got {other:?}"),
    }
    assert_eq!(journal.retained_from(&stream()), Seq(1));
}

/// The documented way to read "everything still retained" — ask for the record
/// before the horizon — works on a truncated stream and degenerates to `None`
/// on an untruncated one.
#[test]
fn reading_from_the_horizon_is_the_way_to_read_everything_retained() {
    let (mut journal, _agg, _seed) = driven(6);

    // Untruncated: the horizon expression is Seq(0), equivalent to `None`.
    let from_horizon = Seq(journal.retained_from(&stream()).0 - 1);
    assert_eq!(from_horizon, Seq(0));
    assert_eq!(
        journal
            .events_since(&stream(), Some(from_horizon))
            .unwrap()
            .len(),
        journal.events_since(&stream(), None).unwrap().len(),
        "on an untruncated stream the two must agree",
    );

    journal.truncate_before(&stream(), Seq(4)).unwrap();

    // Truncated: `None` is refused, the horizon expression still works.
    assert!(journal.events_since(&stream(), None).is_err());
    let from_horizon = Seq(journal.retained_from(&stream()).0 - 1);
    assert_eq!(from_horizon, Seq(3));
    assert_eq!(
        journal
            .events_since(&stream(), Some(from_horizon))
            .unwrap()
            .len(),
        3,
        "records 4..6 are what remains",
    );
}

/// Truncating to a point already below the horizon cannot keep records "from
/// `at` onward" — they are gone — so it must refuse rather than report success
/// while `retained_from` stays put. This matches `record_at` and `fork`, which
/// both treat sequences at or below the horizon as unknown.
#[test]
fn truncating_below_an_existing_horizon_is_refused() {
    let (mut journal, _agg, _seed) = driven(6);
    journal.truncate_before(&stream(), Seq(4)).unwrap();
    assert_eq!(journal.retained_from(&stream()), Seq(4));

    for stale in [Seq(1), Seq(2), Seq(3)] {
        match journal.truncate_before(&stream(), stale) {
            Err(JournalError::UnknownSeq { at }) => assert_eq!(at, stale),
            other => panic!("expected UnknownSeq truncating to {stale:?}, got {other:?}"),
        }
        assert_eq!(
            journal.retained_from(&stream()),
            Seq(4),
            "a refused truncation must not move the horizon",
        );
    }

    // Truncating to exactly the current horizon is the one honest no-op:
    // the stream is already there, so truncation is idempotent.
    journal.truncate_before(&stream(), Seq(4)).unwrap();
    assert_eq!(journal.retained_from(&stream()), Seq(4));
    assert_eq!(journal.head(&stream()), Some(Seq(6)));
}
