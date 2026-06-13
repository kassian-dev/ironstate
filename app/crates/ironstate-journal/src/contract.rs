//! The seven-property journal conformance suite behind `journal_contract_test!`.
//!
//! Every storage adapter is judged against these, and the reference
//! `MemoryJournal` must pass them all. The seven properties: round-trip;
//! position totality & monotonicity; resume identity; fork-position equality;
//! snapshot-vs-head discipline; failed-append atomicity; version tagging.

use crate::journal::{ExecuteError, Journal, JournalError, Seq, Snapshot, VersionedEvent};
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

fn head_pos<A, J>(journal: &J) -> DrawPos
where
    A: AggregateRules,
    J: Journal<A>,
{
    journal.head().map_or(DrawPos(0), |head| {
        journal.entropy_pos(head).expect("position at head")
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
    J: ContractJournal<A>,
    A: AggregateArbitrary + StableHash,
    A::Ctx: CtxEntropy,
{
    let mut journal = J::fresh(genesis.clone());
    let mut aggregate =
        Aggregate::new(genesis).expect("a sampled initial state is in its initial phase");
    let mut steps = Vec::new();
    for step in 0..max_steps {
        if aggregate.phase().is_terminal() {
            break;
        }
        let cmd = sample(A::command_strategy(aggregate.state()), runner);
        let pos = head_pos::<A, J>(&journal);
        let mut ctx = A::test_ctx(Box::new(SeededEntropy::at(seed, pos)), step as u64);
        if let Ok(seq) = execute(&mut journal, &mut aggregate, &cmd, &mut ctx) {
            steps.push((seq, digest128(aggregate.state())));
        }
    }
    (journal, aggregate, steps)
}

/// Run all seven contract properties against `J` for aggregate `A`.
pub fn run_contract<J, A>(cases: u32, max_steps: usize, seed_base: u64)
where
    J: ContractJournal<A>,
    A: AggregateArbitrary + StableHash,
    A::Ctx: CtxEntropy,
{
    let mut runner = seeded_runner(seed_base);
    for case in 0..cases {
        let genesis = sample(A::initial_state_strategy(), &mut runner);
        let seed = run_seed(seed_base, case);
        let (journal, live, steps) = drive::<J, A>(genesis.clone(), &seed, &mut runner, max_steps);

        property_2_positions_total_and_monotonic(&journal, case);
        property_1_round_trip(&journal, &genesis, &steps, case);
        property_7_version_tagging(&journal, case);
        property_3_resume_identity(&journal, live, &seed, &mut runner, case);
        property_4_fork_position_equality(&journal, case);
        property_5_snapshot_vs_head(&journal, &seed, case);

        property_6_failed_append_atomicity::<J, A>(&seed, &mut runner, case);
    }
}

fn property_2_positions_total_and_monotonic<J, A>(journal: &J, case: u32)
where
    A: AggregateRules,
    J: Journal<A>,
{
    let mut previous = DrawPos(0);
    if let Some(head) = journal.head() {
        for seq in 1..=head.0 {
            let pos = journal.entropy_pos(Seq(seq)).unwrap_or_else(|_| {
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

fn property_1_round_trip<J, A>(
    journal: &J,
    genesis: &A,
    steps: &[(Seq, ironstate_aggregate::Digest128)],
    case: u32,
) where
    J: Journal<A>,
    A: AggregateRules + Clone + StableHash,
{
    for (seq, live_digest) in steps {
        let branch = journal.fork(*seq).expect("fork at a recorded Seq");
        let events = branch.events_since(None).expect("events");
        let snapshot = genesis_snapshot(genesis.clone());
        let rebuilt = replay(snapshot, &events).expect("replay");
        assert_eq!(
            digest128(rebuilt.state()),
            *live_digest,
            "property 1: replay did not reproduce the live digest at {seq:?}, case {case}",
        );
    }
}

fn property_7_version_tagging<J, A>(journal: &J, case: u32)
where
    A: AggregateRules,
    J: Journal<A>,
{
    for event in journal.events_since(None).expect("events") {
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
    mut live: Aggregate<A>,
    seed: &Seed,
    runner: &mut TestRunner,
    case: u32,
) where
    J: Journal<A>,
    A: AggregateArbitrary + StableHash,
    A::Ctx: CtxEntropy,
{
    if journal.head().is_none() {
        return;
    }
    let pos = head_pos::<A, J>(journal);
    let cmd = sample(A::command_strategy(live.state()), runner);

    // Resume to head, then handle one command.
    let (mut resumed, _) = resume::<A, J>(journal, seed).expect("resume");
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

fn property_4_fork_position_equality<J, A>(journal: &J, case: u32)
where
    A: AggregateRules,
    J: Journal<A>,
{
    if let Some(head) = journal.head() {
        let at = Seq(head.0.div_ceil(2).max(1));
        let branch = journal.fork(at).expect("fork");
        assert_eq!(
            branch.entropy_pos(at).expect("branch position"),
            journal.entropy_pos(at).expect("main position"),
            "property 4: entropy_pos disagreed at the fork point, case {case}",
        );
        assert_eq!(
            branch.head(),
            Some(at),
            "property 4: a fork's head should sit at the fork point, case {case}",
        );
    }
}

fn property_5_snapshot_vs_head<J, A>(journal: &J, seed: &Seed, case: u32)
where
    A: AggregateRules,
    J: Journal<A>,
{
    if journal.head().is_none() {
        return;
    }
    let pos = head_pos::<A, J>(journal);
    let (_, entropy) = resume::<A, J>(journal, seed).expect("resume");
    assert_eq!(
        entropy.draws(),
        pos,
        "property 5: resume must position entropy at the head, not an earlier snapshot, case {case}",
    );
}

fn property_6_failed_append_atomicity<J, A>(seed: &Seed, runner: &mut TestRunner, case: u32)
where
    J: ContractJournal<A>,
    A: AggregateArbitrary + StableHash,
    A::Ctx: CtxEntropy,
{
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
        match execute(&mut journal, &mut aggregate, &cmd, &mut ctx) {
            Err(ExecuteError::Journal(_)) => {
                assert_eq!(
                    journal.head(),
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
    fn append(&mut self, events: &[A::Event], entropy_pos: DrawPos) -> Result<Seq, JournalError> {
        if self.armed {
            self.armed = false;
            return Err(JournalError::Storage("injected append failure".into()));
        }
        self.inner.append(events, entropy_pos)
    }
    fn entropy_pos(&self, at: Seq) -> Result<DrawPos, JournalError> {
        self.inner.entropy_pos(at)
    }
    fn head(&self) -> Option<Seq> {
        self.inner.head()
    }
    fn events_since(&self, after: Option<Seq>) -> Result<Vec<VersionedEvent<A>>, JournalError> {
        self.inner.events_since(after)
    }
    fn snapshot(&mut self, snapshot: Snapshot<A>) -> Result<(), JournalError> {
        self.inner.snapshot(snapshot)
    }
    fn latest_snapshot(&self) -> Result<Option<Snapshot<A>>, JournalError> {
        self.inner.latest_snapshot()
    }
    fn fork(&self, at: Seq) -> Result<Self, JournalError> {
        Ok(FailNextAppend {
            inner: self.inner.fork(at)?,
            armed: self.armed,
        })
    }
}
