//! Runtime behind the `test!` macro: randomized sequences checked against
//! structural enforcement and declared invariants.

use crate::invariant::Invariant;
use crate::machine::{EventKind, Machine, StateMachine};

/// Parameters parsed from the `test!` macro's `key = value` arguments.
#[derive(Debug, Clone)]
pub struct TestParams {
    /// Number of random event sequences to generate.
    pub cases: u32,
    /// Maximum events per sequence.
    pub max_steps: usize,
    /// Optional fixed seed for a fully reproducible run.
    pub seed: Option<u64>,
}

impl TestParams {
    /// The defaults: 500 cases, up to 100 steps, no fixed seed.
    pub fn new() -> Self {
        Self {
            cases: 500,
            max_steps: 100,
            seed: None,
        }
    }
}

impl Default for TestParams {
    fn default() -> Self {
        Self::new()
    }
}

/// Autoref-specialization machinery for picking up declared invariants.
///
/// A machine with an `Invariants` impl resolves to `ViaImpl` (which requires
/// it); one without falls back to `ViaNone`. The choice must be made where
/// the concrete type is known — the `test!` macro — because a generic function
/// could not see the `Invariants` impl. The macro calls `(&&Probe).collect()`.
pub mod probe {
    use crate::invariant::{Invariant, Invariants};
    use crate::machine::StateMachine;
    use core::marker::PhantomData;

    /// Carrier the autoref trick dispatches on.
    pub struct Probe<S>(pub PhantomData<S>);

    /// Fallback for a machine that declares no invariants.
    pub trait ViaNone<S: StateMachine> {
        /// No declared invariants.
        fn collect(&self) -> Vec<Invariant<S, S::Event>> {
            Vec::new()
        }
    }
    impl<S: StateMachine> ViaNone<S> for Probe<S> {}

    /// Selected when the machine implements `Invariants`.
    pub trait ViaImpl<S: StateMachine> {
        /// The machine's declared invariants.
        fn collect(&self) -> Vec<Invariant<S, S::Event>>;
    }
    impl<S: StateMachine + Invariants> ViaImpl<S> for &Probe<S> {
        fn collect(&self) -> Vec<Invariant<S, S::Event>> {
            <S as Invariants>::invariants()
        }
    }
}

/// Run randomized property testing for a machine.
///
/// Generates random event sequences and, after every step, asserts that the
/// supplied invariants hold and nothing panicked. Structural enforcement is
/// guaranteed by `apply` itself, so a violation here means a broken invariant.
pub fn run<S: StateMachine>(params: TestParams, invariants: Vec<Invariant<S, S::Event>>) {
    use proptest::prelude::*;
    use proptest::test_runner::{Config, TestError, TestRng, TestRunner};

    let events = S::Event::event_variants();
    assert!(
        !events.is_empty(),
        "`{}` has no event variants to drive testing",
        std::any::type_name::<S>()
    );

    let config = Config {
        cases: params.cases,
        ..Config::default()
    };
    let mut runner = match params.seed {
        Some(seed) => {
            let mut bytes = [0u8; 32];
            bytes[..8].copy_from_slice(&seed.to_le_bytes());
            TestRunner::new_with_rng(
                config,
                TestRng::from_seed(proptest::test_runner::RngAlgorithm::ChaCha, &bytes),
            )
        }
        None => TestRunner::new(config),
    };

    let strategy = proptest::collection::vec(0..events.len(), 0..=params.max_steps);
    let result = runner.run(&strategy, |indices| {
        let mut machine = Machine::<S>::new();
        for idx in indices {
            let event = events[idx].clone();
            let before = machine.state().clone();
            let after = machine.apply(event.clone()).ok();
            for invariant in &invariants {
                if !invariant.holds(&before, &event, &after) {
                    return Err(TestCaseError::fail(format!(
                        "invariant violated: {}\n  before: {before:?}\n  event:  {event:?}\n  after:  {after:?}",
                        invariant.description(),
                    )));
                }
            }
        }
        Ok(())
    });

    if let Err(err) = result {
        match err {
            TestError::Fail(reason, case) => panic!(
                "`{}` failed an invariant.\n{reason}\nminimal failing sequence: {case:?}",
                std::any::type_name::<S>()
            ),
            TestError::Abort(reason) => {
                panic!("`{}` test aborted: {reason}", std::any::type_name::<S>())
            }
        }
    }
}
