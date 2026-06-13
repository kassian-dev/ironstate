//! Public deterministic-simulation testkit behind `scenario_test!`.
//!
//! One seed drives everything: the command stream and a fault schedule
//! interleaved with it. The property is that **faults are invisible to
//! outcomes** — a fault-injected run reaches the same final `Digest128` as a
//! fault-free [`ReferenceRun`] over the same commands. The pieces are public so
//! a consumer's own deterministic-simulation harness can reuse them.

use crate::journal::{ExecuteError, Journal, JournalError, Seq, Snapshot, VersionedEvent};
use crate::memory::MemoryJournal;
use crate::replay::{execute, resume};
use ironstate::StateMachine;
use ironstate_aggregate::{
    Aggregate, AggregateArbitrary, AggregateRules, CtxEntropy, Digest128, DrawPos, EntropySource,
    Seed, SeededEntropy, StableHash, digest128,
};
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};

/// A single fault the schedule can interleave with the command stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// The next append fails (and is retried).
    AppendFailure,
    /// A source delivery is duplicated (subscription harnesses).
    DuplicateDelivery {
        /// The sequence number redelivered.
        of: Seq,
    },
    /// Source deliveries are re-ordered within a window (subscription harnesses).
    ReorderDelivery {
        /// The reorder window.
        window: u32,
    },
    /// Drop the in-memory aggregate and `resume` from the journal.
    CrashResume {
        /// The sequence number the crash happened at.
        at: Seq,
    },
    /// Fork and continue on both branches.
    ForkContinue {
        /// The sequence number forked at.
        at: Seq,
    },
    /// Plant an older-version record mid-stream (versioning harnesses).
    PlantOldVersion {
        /// The version to plant.
        version: u32,
    },
}

/// A seeded, per-step schedule of faults — derived from one entropy stream, so
/// it reproduces from a seed and can be narrowed when shrinking a failure.
pub struct FaultSchedule {
    per_step: Vec<Option<Fault>>,
}

impl FaultSchedule {
    /// Generate a schedule for `steps` steps. Roughly one step in three gets a
    /// fault, chosen among those a single-aggregate run can apply.
    pub fn generate(entropy: &mut dyn EntropySource, steps: usize) -> Self {
        let mut per_step = Vec::with_capacity(steps);
        for _ in 0..steps {
            let fault = if entropy.draw_below(1, 3) {
                Some(match entropy.draw_range(0..3) {
                    0 => Fault::AppendFailure,
                    1 => Fault::CrashResume { at: Seq(0) },
                    _ => Fault::ForkContinue { at: Seq(0) },
                })
            } else {
                None
            };
            per_step.push(fault);
        }
        Self { per_step }
    }

    /// The fault scheduled at `step`, if any.
    pub fn at(&self, step: usize) -> Option<&Fault> {
        self.per_step.get(step).and_then(Option::as_ref)
    }
}

/// Wraps any [`Journal`] so a harness can inject an append failure on demand.
pub struct FaultInjector<J, A: AggregateRules> {
    inner: J,
    fail_next_append: bool,
    _marker: core::marker::PhantomData<fn(A)>,
}

impl<J: Journal<A>, A: AggregateRules> FaultInjector<J, A> {
    /// Wrap an inner journal.
    pub fn new(inner: J) -> Self {
        Self {
            inner,
            fail_next_append: false,
            _marker: core::marker::PhantomData,
        }
    }
    /// Make the next `append` fail once.
    pub fn arm_append_failure(&mut self) {
        self.fail_next_append = true;
    }
    /// Clear a pending append failure that was never triggered.
    pub fn disarm(&mut self) {
        self.fail_next_append = false;
    }
    /// Recover the inner journal.
    pub fn into_inner(self) -> J {
        self.inner
    }
}

impl<J: Journal<A>, A: AggregateRules> Journal<A> for FaultInjector<J, A> {
    fn append(&mut self, events: &[A::Event], entropy_pos: DrawPos) -> Result<Seq, JournalError> {
        if self.fail_next_append {
            self.fail_next_append = false;
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
        Ok(FaultInjector {
            inner: self.inner.fork(at)?,
            fail_next_append: false,
            _marker: core::marker::PhantomData,
        })
    }
}

/// A fault-free run over a recorded command stream, the oracle a faulted run is
/// compared against.
pub struct ReferenceRun<A: AggregateArbitrary> {
    commands: Vec<A::Command>,
    final_digest: Digest128,
}

impl<A: AggregateArbitrary + StableHash> ReferenceRun<A>
where
    A::Ctx: CtxEntropy,
{
    /// Sample and run a command stream fault-free, recording the commands (so a
    /// faulted run can replay exactly them) and the final state digest.
    fn record(genesis: A, seed: &Seed, runner: &mut TestRunner, max_steps: usize) -> Self {
        let mut journal = MemoryJournal::new(genesis.clone());
        let mut aggregate = Aggregate::new(genesis).expect("initial");
        let mut commands = Vec::new();
        for _ in 0..max_steps {
            if aggregate.phase().is_terminal() {
                break;
            }
            let cmd = sample(A::command_strategy(aggregate.state()), runner);
            exec_step(&mut journal, &mut aggregate, &cmd, seed).ok();
            commands.push(cmd);
        }
        Self {
            commands,
            final_digest: digest128(aggregate.state()),
        }
    }

    /// The commands the reference run drove.
    pub fn commands(&self) -> &[A::Command] {
        &self.commands
    }

    /// The final state digest of the fault-free run.
    pub fn final_digest(&self) -> Digest128 {
        self.final_digest
    }
}

/// Run the seeded whole-tier simulation: for each case, drive a command stream
/// fault-free and again under a fault schedule, asserting the faulted run
/// reaches the same final digest — faults invisible to outcomes.
pub fn run_scenario<A>(cases: u32, max_steps: usize, seed_base: u64)
where
    A: AggregateArbitrary + StableHash,
    A::Ctx: CtxEntropy,
{
    let mut runner = seeded_runner(seed_base);
    for case in 0..cases {
        let genesis = sample(A::initial_state_strategy(), &mut runner);
        let seed = run_seed(seed_base, case, 0);

        let reference = ReferenceRun::<A>::record(genesis.clone(), &seed, &mut runner, max_steps);

        let mut schedule_entropy = SeededEntropy::at(&run_seed(seed_base, case, 1), DrawPos(0));
        let schedule = FaultSchedule::generate(&mut schedule_entropy, reference.commands().len());

        let faulted = run_faulted::<A>(genesis, &seed, reference.commands(), &schedule, case);

        assert_eq!(
            faulted,
            reference.final_digest(),
            "[sampled] scenario: a faulted run diverged from the fault-free run — \
             faults must be invisible to outcomes (seed {seed_base:#x}, case {case})",
        );
    }
}

fn run_faulted<A>(
    genesis: A,
    seed: &Seed,
    commands: &[A::Command],
    schedule: &FaultSchedule,
    case: u32,
) -> Digest128
where
    A: AggregateArbitrary + StableHash,
    A::Ctx: CtxEntropy,
{
    let mut journal = FaultInjector::new(MemoryJournal::new(genesis.clone()));
    let mut aggregate = Aggregate::new(genesis).expect("initial");

    for (step, cmd) in commands.iter().enumerate() {
        if aggregate.phase().is_terminal() {
            break;
        }
        match schedule.at(step) {
            Some(Fault::AppendFailure) => {
                journal.arm_append_failure();
                match exec_step(&mut journal, &mut aggregate, cmd, seed) {
                    // The append failed as injected; rewound, so retry reproduces it.
                    Err(ExecuteError::Journal(_)) => {
                        exec_step(&mut journal, &mut aggregate, cmd, seed).ok();
                    }
                    // The command never reached the append; clear the arming.
                    _ => journal.disarm(),
                }
            }
            Some(Fault::CrashResume { .. }) => {
                exec_step(&mut journal, &mut aggregate, cmd, seed).ok();
                let (resumed, _) = resume::<A, _>(&journal, seed).expect("resume after crash");
                aggregate = resumed;
            }
            Some(Fault::ForkContinue { .. }) => {
                exec_step(&mut journal, &mut aggregate, cmd, seed).ok();
                if let Some(head) = journal.head() {
                    let branch = journal.fork(head).expect("fork");
                    let (forked, _) = resume::<A, _>(&branch, seed).expect("resume on fork");
                    assert_eq!(
                        digest128(forked.state()),
                        digest128(aggregate.state()),
                        "scenario: a fork did not reproduce the live state, case {case}",
                    );
                }
            }
            _ => {
                exec_step(&mut journal, &mut aggregate, cmd, seed).ok();
            }
        }
    }
    digest128(aggregate.state())
}

fn exec_step<A, J>(
    journal: &mut J,
    aggregate: &mut Aggregate<A>,
    cmd: &A::Command,
    seed: &Seed,
) -> Result<Seq, ExecuteError<A>>
where
    A: AggregateArbitrary,
    A::Ctx: CtxEntropy,
    J: Journal<A>,
{
    let pos = journal.head().map_or(DrawPos(0), |head| {
        journal.entropy_pos(head).expect("position")
    });
    let mut ctx = A::test_ctx(Box::new(SeededEntropy::at(seed, pos)), 0);
    execute(journal, aggregate, cmd, &mut ctx)
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

fn run_seed(seed: u64, case: u32, stream: u8) -> Seed {
    let mut bytes = [0u8; 32];
    bytes[..8].copy_from_slice(&seed.to_le_bytes());
    bytes[8..12].copy_from_slice(&case.to_le_bytes());
    bytes[12] = stream;
    Seed(bytes)
}
