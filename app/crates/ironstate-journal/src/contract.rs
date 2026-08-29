//! The journal conformance suite behind `journal_contract_test!`.
//!
//! Every storage adapter is judged against these, and the reference
//! `MemoryJournal` must pass them all.
//!
//! Eight properties apply to every journal: round-trip; position totality &
//! monotonicity; resume identity; snapshot-vs-head discipline; failed-append
//! atomicity; version tagging; **stream independence**; and **out-of-range
//! addressing**. Two more — round-trip at each recorded step, and
//! fork-position equality — need [`ForkableJournal`] and so are run only by
//! [`run_contract_forkable`].

use crate::journal::{
    ExecuteError, ForkableJournal, Journal, JournalError, RetainableJournal, Seq, Snapshot,
    StreamId, VersionedEvent,
};
use crate::memory::MemoryJournal;
use crate::replay::{execute, replay, resume};
use ironstate::StateMachine;
use ironstate_aggregate::{
    Aggregate, AggregateArbitrary, AggregateRules, CtxEntropy, DrawPos, EntropySource, Seed,
    SeededEntropy, StableHash, digest128,
};
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};

/// A journal an adapter author can construct freshly for the contract suite.
///
/// # Journals with a real transaction
///
/// The suite drives adapters through [`execute`], so every entry point is
/// bound `for<'a> Journal<A, Tx<'a> = ()>`: a journal whose `Tx` is an actual
/// database transaction cannot be run through
/// [`journal_contract_test!`](crate::journal_contract_test) as it stands,
/// because the harness has no way to mint and resolve a unit of work.
///
/// Until it does, hold such an adapter to the contract with a twin over the
/// same storage whose `Tx` is `()` — the pattern the `async-store` example
/// already uses for a store that cannot implement the synchronous trait at
/// all. This is a limitation of the harness, not of the adapter. The rollback
/// behaviour only a real `Tx` can exhibit is covered separately, in
/// `tests/transactional.rs`.
pub trait ContractJournal<A: AggregateRules + Clone>: Journal<A> {
    /// A fresh, empty journal seeded with the aggregate's genesis state.
    fn fresh(genesis: A) -> Self;
}

impl<A: AggregateRules + Clone> ContractJournal<A> for MemoryJournal<A> {
    fn fresh(genesis: A) -> Self {
        MemoryJournal::new(genesis)
    }
}

fn sample<S: Strategy>(strategy: S, runner: &mut TestRunner) -> S::Value {
    strategy.new_tree(runner).expect("strategy").current()
}

fn seeded_runner(seed: u64) -> TestRunner {
    let mut bytes = [0u8; 32];
    bytes[..8].copy_from_slice(&seed.to_le_bytes());
    TestRunner::new_with_rng(
        Config {
            cases: 1,
            ..Config::default()
        },
        TestRng::from_seed(RngAlgorithm::ChaCha, &bytes),
    )
}

fn run_seed(seed: u64, case: u32) -> Seed {
    let mut bytes = [0u8; 32];
    bytes[..8].copy_from_slice(&seed.to_le_bytes());
    bytes[8..12].copy_from_slice(&case.to_le_bytes());
    Seed(bytes)
}

/// The stream the suite drives. A second, independent stream is used by the
/// stream-independence property.
fn test_stream() -> StreamId {
    StreamId::new("contract")
}

/// A stream that must never be touched by appends to [`test_stream`].
fn other_stream() -> StreamId {
    StreamId::new("contract-other")
}

fn head_pos<A, J>(journal: &J, stream: &StreamId) -> DrawPos
where
    A: AggregateRules,
    J: Journal<A>,
{
    journal.head(stream).map_or(DrawPos(0), |head| {
        journal.entropy_pos(stream, head).expect("position at head")
    })
}

/// Drive a fresh journal with a generated command stream, recording the live
/// state digest after each successful append.
fn drive<J, A>(
    genesis: A,
    seed: &Seed,
    runner: &mut TestRunner,
    max_steps: usize,
) -> (J, Aggregate<A>, Vec<(Seq, ironstate_aggregate::Digest128)>)
where
    J: ContractJournal<A> + for<'a> Journal<A, Tx<'a> = ()>,
    A: AggregateArbitrary + StableHash,
    A::Ctx: CtxEntropy,
{
    let stream = test_stream();
    let mut journal = J::fresh(genesis.clone());
    let mut aggregate =
        Aggregate::new(genesis).expect("a sampled initial state is in its initial phase");
    let mut steps = Vec::new();
    for step in 0..max_steps {
        if aggregate.phase().is_terminal() {
            break;
        }
        let cmd = sample(A::command_strategy(aggregate.state()), runner);
        let pos = head_pos::<A, J>(&journal, &stream);
        let mut ctx = A::test_ctx(Box::new(SeededEntropy::at(seed, pos)), step as u64);
        if let Ok(seq) = execute(&mut journal, &stream, &mut aggregate, &cmd, &mut ctx) {
            steps.push((seq, digest128(aggregate.state())));
        }
    }
    (journal, aggregate, steps)
}

/// Run the eight properties every journal must satisfy, whether or not it can
/// fork.
///
/// # Panics
///
/// Panics with a `[proven]`/property-numbered message naming the first
/// violation found.
pub fn run_contract<J, A>(cases: u32, max_steps: usize, seed_base: u64)
where
    J: ContractJournal<A> + for<'a> Journal<A, Tx<'a> = ()>,
    A: AggregateArbitrary + StableHash,
    A::Ctx: CtxEntropy,
{
    let stream = test_stream();
    let mut runner = seeded_runner(seed_base);
    for case in 0..cases {
        let genesis = sample(A::initial_state_strategy(), &mut runner);
        let seed = run_seed(seed_base, case);
        let (journal, live, steps) = drive::<J, A>(genesis.clone(), &seed, &mut runner, max_steps);

        property_2_positions_total_and_monotonic(&journal, &stream, case);
        property_1_round_trip(&journal, &stream, &genesis, &steps, case);
        property_7_version_tagging(&journal, &stream, case);
        property_3_resume_identity(&journal, &stream, live, &seed, &mut runner, case);
        property_5_snapshot_vs_head(&journal, &stream, &seed, case);
        property_9_out_of_range_seq(&journal, &stream, case);

        property_6_failed_append_atomicity::<J, A>(&seed, &mut runner, case);
        property_8_stream_independence::<J, A>(genesis, &seed, &mut runner, max_steps, case);
    }
}

/// Run the retention property against a journal that can truncate.
///
/// # Panics
///
/// Panics with a property-numbered message if truncating at a snapshot boundary
/// changes what the stream resumes to.
pub fn run_contract_retainable<J, A>(cases: u32, max_steps: usize, seed_base: u64)
where
    J: ContractJournal<A> + RetainableJournal<A> + for<'a> Journal<A, Tx<'a> = ()>,
    A: AggregateArbitrary + StableHash,
    A::Ctx: CtxEntropy,
{
    let stream = test_stream();
    let mut runner = seeded_runner(seed_base);
    for case in 0..cases {
        let genesis = sample(A::initial_state_strategy(), &mut runner);
        let seed = run_seed(seed_base, case);
        let (mut journal, _live, _steps) =
            drive::<J, A>(genesis.clone(), &seed, &mut runner, max_steps);
        property_11_truncation_preserves_resume(&mut journal, &stream, &seed, case);
    }
}

/// Run every property, including the two that need [`ForkableJournal`].
///
/// # Panics
///
/// Panics with a `[proven]`/property-numbered message naming the first
/// violation found.
pub fn run_contract_forkable<J, A>(cases: u32, max_steps: usize, seed_base: u64)
where
    J: ContractJournal<A> + ForkableJournal<A> + for<'a> Journal<A, Tx<'a> = ()>,
    A: AggregateArbitrary + StableHash,
    A::Ctx: CtxEntropy,
{
    let stream = test_stream();
    let mut runner = seeded_runner(seed_base);
    for case in 0..cases {
        let genesis = sample(A::initial_state_strategy(), &mut runner);
        let seed = run_seed(seed_base, case);
        let (journal, live, steps) = drive::<J, A>(genesis.clone(), &seed, &mut runner, max_steps);

        property_2_positions_total_and_monotonic(&journal, &stream, case);
        property_1_round_trip(&journal, &stream, &genesis, &steps, case);
        property_7_version_tagging(&journal, &stream, case);
        property_3_resume_identity(&journal, &stream, live, &seed, &mut runner, case);
        property_5_snapshot_vs_head(&journal, &stream, &seed, case);
        property_9_out_of_range_seq(&journal, &stream, case);

        property_6_failed_append_atomicity::<J, A>(&seed, &mut runner, case);
        property_8_stream_independence::<J, A>(
            genesis.clone(),
            &seed,
            &mut runner,
            max_steps,
            case,
        );

        // The branching properties, in the same pass over the same histories —
        // driving a second, separately-seeded set would double the cost and make
        // a fork failure irreproducible against the case just reported.
        property_1_round_trip_at_each_step(&journal, &stream, &genesis, &steps, case);
        property_4_fork_position_equality(&journal, &stream, case);
    }
}

fn property_2_positions_total_and_monotonic<J, A>(journal: &J, stream: &StreamId, case: u32)
where
    A: AggregateRules,
    J: Journal<A>,
{
    let mut previous = DrawPos(0);
    if let Some(head) = journal.head(stream) {
        for seq in 1..=head.0 {
            let pos = journal.entropy_pos(stream, Seq(seq)).unwrap_or_else(|_| {
                panic!("[proven] property 2: entropy_pos undefined at Seq({seq}), case {case}")
            });
            assert!(
                pos >= previous,
                "[proven] property 2: entropy_pos decreased at Seq({seq}), case {case}",
            );
            previous = pos;
        }
    }
}

/// Property 1 — **round trip**. Replaying the log reproduces the live state.
///
/// The single most important thing a journal does, so it must hold for every
/// adapter, not only forkable ones: it is expressed against `events_since`
/// alone. [`property_1_round_trip_at_each_step`] additionally checks every
/// intermediate point, which does need branching.
fn property_1_round_trip<J, A>(
    journal: &J,
    stream: &StreamId,
    genesis: &A,
    steps: &[(Seq, ironstate_aggregate::Digest128)],
    case: u32,
) where
    J: Journal<A>,
    A: AggregateRules + Clone + StableHash,
{
    let Some((_, live_digest)) = steps.last() else {
        return;
    };
    let events = journal.events_since(stream, None).expect("events");
    let rebuilt = replay(genesis_snapshot(genesis.clone()), &events).expect("replay");
    assert_eq!(
        digest128(rebuilt.state()),
        *live_digest,
        "property 1: replay of the whole log did not reproduce the live digest, case {case}",
    );
}

/// Property 1b — round trip at **every** recorded step, which needs a branch to
/// isolate each prefix.
fn property_1_round_trip_at_each_step<J, A>(
    journal: &J,
    stream: &StreamId,
    genesis: &A,
    steps: &[(Seq, ironstate_aggregate::Digest128)],
    case: u32,
) where
    J: ForkableJournal<A>,
    A: AggregateRules + Clone + StableHash,
{
    for (seq, live_digest) in steps {
        let branch = journal.fork(stream, *seq).expect("fork at a recorded Seq");
        let events = branch.events_since(stream, None).expect("events");
        let snapshot = genesis_snapshot(genesis.clone());
        let rebuilt = replay(snapshot, &events).expect("replay");
        assert_eq!(
            digest128(rebuilt.state()),
            *live_digest,
            "property 1b: replay did not reproduce the live digest at {seq:?}, case {case}",
        );
    }
}

fn property_7_version_tagging<J, A>(journal: &J, stream: &StreamId, case: u32)
where
    A: AggregateRules,
    J: Journal<A>,
{
    for event in journal.events_since(stream, None).expect("events") {
        let VersionedEvent {
            type_name, version, ..
        } = event;
        assert!(
            !type_name.is_empty(),
            "[proven] property 7: a record is missing its type name, case {case}",
        );
        assert!(
            version >= 1,
            "[proven] property 7: a record is missing its version, case {case}",
        );
    }
}

fn property_3_resume_identity<J, A>(
    journal: &J,
    stream: &StreamId,
    mut live: Aggregate<A>,
    seed: &Seed,
    runner: &mut TestRunner,
    case: u32,
) where
    J: Journal<A>,
    A: AggregateArbitrary + StableHash,
    A::Ctx: CtxEntropy,
{
    if journal.head(stream).is_none() {
        return;
    }
    let pos = head_pos::<A, J>(journal, stream);
    let cmd = sample(A::command_strategy(live.state()), runner);

    // Resume to head, then handle one command.
    let (mut resumed, _) = resume::<A, J>(journal, stream, seed).expect("resume");
    let mut ctx_r = A::test_ctx(Box::new(SeededEntropy::at(seed, pos)), 0);
    let _ = resumed.handle(&cmd, &mut ctx_r);

    // The live aggregate handles the same command from the same position.
    let mut ctx_l = A::test_ctx(Box::new(SeededEntropy::at(seed, pos)), 0);
    let _ = live.handle(&cmd, &mut ctx_l);

    assert_eq!(
        digest128(resumed.state()),
        digest128(live.state()),
        "property 3: resume-to-head then handle diverged from the live handle, case {case}",
    );
}

fn property_4_fork_position_equality<J, A>(journal: &J, stream: &StreamId, case: u32)
where
    A: AggregateRules,
    J: ForkableJournal<A>,
{
    if let Some(head) = journal.head(stream) {
        let at = Seq(head.0.div_ceil(2).max(1));
        let branch = journal.fork(stream, at).expect("fork");
        assert_eq!(
            branch.entropy_pos(stream, at).expect("branch position"),
            journal.entropy_pos(stream, at).expect("main position"),
            "property 4: entropy_pos disagreed at the fork point, case {case}",
        );
        assert_eq!(
            branch.head(stream),
            Some(at),
            "property 4: a fork's head should sit at the fork point, case {case}",
        );
    }
}

fn property_5_snapshot_vs_head<J, A>(journal: &J, stream: &StreamId, seed: &Seed, case: u32)
where
    A: AggregateRules,
    J: Journal<A>,
{
    if journal.head(stream).is_none() {
        return;
    }
    let pos = head_pos::<A, J>(journal, stream);
    let (_, entropy) = resume::<A, J>(journal, stream, seed).expect("resume");
    assert_eq!(
        entropy.draws(),
        pos,
        "property 5: resume must position entropy at the head, not an earlier snapshot, case {case}",
    );
}

fn property_6_failed_append_atomicity<J, A>(seed: &Seed, runner: &mut TestRunner, case: u32)
where
    J: ContractJournal<A> + for<'a> Journal<A, Tx<'a> = ()>,
    A: AggregateArbitrary + StableHash,
    A::Ctx: CtxEntropy,
{
    let stream = test_stream();
    let genesis = sample(A::initial_state_strategy(), runner);
    let mut journal = FailNextAppend {
        inner: J::fresh(genesis.clone()),
        armed: true,
    };
    let mut aggregate = Aggregate::new(genesis).expect("initial");
    let before = digest128(aggregate.state());

    // Find a command that reaches the (failing) append rather than being rejected.
    for _ in 0..8 {
        if aggregate.phase().is_terminal() {
            return;
        }
        let cmd = sample(A::command_strategy(aggregate.state()), runner);
        let mut ctx = A::test_ctx(Box::new(SeededEntropy::at(seed, DrawPos(0))), 0);
        match execute(&mut journal, &stream, &mut aggregate, &cmd, &mut ctx) {
            Err(ExecuteError::Journal(_)) => {
                assert_eq!(
                    journal.head(&stream),
                    None,
                    "property 6: a failed append journaled something, case {case}"
                );
                assert_eq!(
                    digest128(aggregate.state()),
                    before,
                    "property 6: a failed append mutated the state, case {case}",
                );
                let pos = ctx.entropy_mut().map_or(DrawPos(0), |e| e.draws());
                assert_eq!(
                    pos,
                    DrawPos(0),
                    "property 6: a failed append left the entropy advanced, case {case}"
                );
                return;
            }
            // A structural/domain rejection never reached the append; try again.
            Err(ExecuteError::Rejected(_)) => {}
            // Armed, so the first append must fail; an Ok means the wrapper is wrong.
            Ok(_) => unreachable!("FailNextAppend was armed"),
        }
    }
}

/// Property 11 — **truncation preserves resume**. After truncating at a
/// snapshot boundary, the stream resumes to a bit-identical aggregate and the
/// same entropy position.
///
/// This is what makes retention safe to offer at all: dropping history must be
/// invisible to everything downstream of the snapshot it was taken against.
fn property_11_truncation_preserves_resume<J, A>(
    journal: &mut J,
    stream: &StreamId,
    seed: &Seed,
    case: u32,
) where
    J: RetainableJournal<A> + for<'a> Journal<A, Tx<'a> = ()>,
    A: AggregateRules + Clone + StableHash,
{
    let Some(head) = journal.head(stream) else {
        return;
    };
    let Ok((before, entropy_before)) = resume::<A, J>(journal, stream, seed) else {
        return;
    };

    // Snapshot the resumed state at the head, then drop everything before the
    // midpoint — a truncation the snapshot fully covers.
    let snapshot = Snapshot {
        state: before.state().clone(),
        schema_version: 0,
        at: head,
        entropy_pos: entropy_before.draws(),
    };
    journal
        .snapshot_in(&mut (), stream, snapshot)
        .expect("snapshot");

    let at = Seq(head.0.div_ceil(2).max(1));
    journal
        .truncate_before(stream, at)
        .expect("truncating below a snapshot taken at the head must be allowed");

    // Positions stay total and monotonic over what remains, and everything below
    // the horizon is now out of range — the new failure mode retention adds,
    // which property 2 and property 10 cannot see on an untruncated journal.
    let horizon = journal.retained_from(stream);
    let mut previous = DrawPos(0);
    for seq in horizon.0..=head.0 {
        let pos = journal.entropy_pos(stream, Seq(seq)).unwrap_or_else(|_| {
            panic!(
                "[proven] property 11: entropy_pos undefined at retained Seq({seq}), case {case}"
            )
        });
        assert!(
            pos >= previous,
            "[proven] property 11: entropy_pos decreased at Seq({seq}), case {case}",
        );
        previous = pos;
    }
    for gone in 1..horizon.0 {
        assert!(
            matches!(
                journal.entropy_pos(stream, Seq(gone)),
                Err(JournalError::UnknownSeq { .. })
            ),
            "[proven] property 11: truncated Seq({gone}) must be UnknownSeq, case {case}",
        );
    }

    let (after, entropy_after) = resume::<A, J>(journal, stream, seed).expect("resume after");
    assert_eq!(
        digest128(after.state()),
        digest128(before.state()),
        "property 11: truncation changed what the stream resumes to, case {case}",
    );
    assert_eq!(
        entropy_after.draws(),
        entropy_before.draws(),
        "property 11: truncation moved the resume entropy position, case {case}",
    );
    assert_eq!(
        journal.retained_from(stream),
        at,
        "property 11: retained_from must report the new horizon, case {case}",
    );
}

/// Property 8 — **stream independence**. Appends to one stream never move
/// another stream's head, entropy position, or snapshot.
///
/// This is the property a hand-rolled stream-routing layer above a
/// single-stream journal gets wrong, which is why it is part of the contract
/// rather than left to adapter authors.
fn property_8_stream_independence<J, A>(
    genesis: A,
    seed: &Seed,
    runner: &mut TestRunner,
    max_steps: usize,
    case: u32,
) where
    J: ContractJournal<A> + for<'a> Journal<A, Tx<'a> = ()>,
    A: AggregateArbitrary + StableHash,
    A::Ctx: CtxEntropy,
{
    let driven = test_stream();
    let untouched = other_stream();

    let mut journal = J::fresh(genesis.clone());
    let mut aggregate =
        Aggregate::new(genesis).expect("a sampled initial state is in its initial phase");

    // Whatever the untouched stream reports before any append, it must still
    // report after every append to the other stream.
    let before_head = journal.head(&untouched);
    let before_events = journal
        .events_since(&untouched, None)
        .expect("events")
        .len();
    let before_snapshot = journal
        .latest_snapshot(&untouched)
        .expect("snapshot")
        .map(|s| (s.at, s.entropy_pos));

    for step in 0..max_steps {
        if aggregate.phase().is_terminal() {
            break;
        }
        let cmd = sample(A::command_strategy(aggregate.state()), runner);
        let pos = head_pos::<A, J>(&journal, &driven);
        let mut ctx = A::test_ctx(Box::new(SeededEntropy::at(seed, pos)), step as u64);
        let _ = execute(&mut journal, &driven, &mut aggregate, &cmd, &mut ctx);

        assert_eq!(
            journal.head(&untouched),
            before_head,
            "[proven] property 8: an append to one stream moved another's head, case {case}",
        );
    }

    assert_eq!(
        journal
            .events_since(&untouched, None)
            .expect("events")
            .len(),
        before_events,
        "[proven] property 8: an append to one stream added events to another, case {case}",
    );
    assert_eq!(
        journal
            .entropy_pos(&untouched, Seq(0))
            .expect("genesis position"),
        DrawPos(0),
        "[proven] property 8: an append to one stream moved another's entropy, case {case}",
    );
    assert_eq!(
        journal
            .latest_snapshot(&untouched)
            .expect("snapshot")
            .map(|s| (s.at, s.entropy_pos)),
        before_snapshot,
        "[proven] property 8: an append to one stream moved another's snapshot, case {case}",
    );
}

/// Property 9 — **out-of-range addressing**. A `Seq` past the head must yield
/// [`JournalError::UnknownSeq`], never a different record's position.
///
/// `Seq` is a public tuple struct, so a caller can construct any value. On a
/// 32-bit target a naive `as usize` cast truncates, which can turn an
/// out-of-range `Seq` into a valid index — a wrong answer rather than an error.
fn property_9_out_of_range_seq<J, A>(journal: &J, stream: &StreamId, case: u32)
where
    A: AggregateRules,
    J: Journal<A>,
{
    let head = journal.head(stream).map_or(0, |h| h.0);
    for probe in [head + 1, head + 2, u64::from(u32::MAX) + head + 1, u64::MAX] {
        match journal.entropy_pos(stream, Seq(probe)) {
            Err(JournalError::UnknownSeq { at }) => assert_eq!(
                at,
                Seq(probe),
                "[proven] property 9: UnknownSeq reported the wrong Seq, case {case}",
            ),
            Err(other) => panic!(
                "[proven] property 9: out-of-range Seq({probe}) gave {other:?} \
                 rather than UnknownSeq, case {case}"
            ),
            Ok(pos) => panic!(
                "[proven] property 9: out-of-range Seq({probe}) returned a position \
                 ({pos:?}) instead of UnknownSeq, case {case}"
            ),
        }
    }
}

fn genesis_snapshot<A: AggregateRules>(state: A) -> Snapshot<A> {
    Snapshot {
        state,
        schema_version: 0,
        at: Seq(0),
        entropy_pos: DrawPos(0),
    }
}

/// A journal wrapper that fails the next `append`, for property 6.
struct FailNextAppend<J> {
    inner: J,
    armed: bool,
}

impl<A: AggregateRules, J: Journal<A>> Journal<A> for FailNextAppend<J> {
    type Tx<'a> = J::Tx<'a>;

    fn append_in(
        &mut self,
        tx: &mut Self::Tx<'_>,
        stream: &StreamId,
        events: &[A::Event],
        entropy_pos: DrawPos,
    ) -> Result<Seq, JournalError> {
        if self.armed {
            self.armed = false;
            return Err(JournalError::Storage("injected append failure".into()));
        }
        self.inner.append_in(tx, stream, events, entropy_pos)
    }
    fn snapshot_in(
        &mut self,
        tx: &mut Self::Tx<'_>,
        stream: &StreamId,
        snapshot: Snapshot<A>,
    ) -> Result<(), JournalError> {
        self.inner.snapshot_in(tx, stream, snapshot)
    }
    fn entropy_pos(&self, stream: &StreamId, at: Seq) -> Result<DrawPos, JournalError> {
        self.inner.entropy_pos(stream, at)
    }
    fn head(&self, stream: &StreamId) -> Option<Seq> {
        self.inner.head(stream)
    }
    fn events_since(
        &self,
        stream: &StreamId,
        after: Option<Seq>,
    ) -> Result<Vec<VersionedEvent<A>>, JournalError> {
        self.inner.events_since(stream, after)
    }
    fn latest_snapshot(&self, stream: &StreamId) -> Result<Option<Snapshot<A>>, JournalError> {
        self.inner.latest_snapshot(stream)
    }
}

impl<A: AggregateRules, J: ForkableJournal<A>> ForkableJournal<A> for FailNextAppend<J> {
    fn fork(&self, stream: &StreamId, at: Seq) -> Result<Self, JournalError> {
        Ok(FailNextAppend {
            inner: self.inner.fork(stream, at)?,
            armed: self.armed,
        })
    }
}
