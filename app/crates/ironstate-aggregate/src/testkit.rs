//! Property-test drivers shared by `test!`, `determinism_test!`, and
//! `leak_test!`.
//!
//! All three sample an initial state, then repeatedly sample a command, build a
//! context over a seeded entropy stream, and `handle` it — rejections recorded
//! and skipped, events applied. On top of that one driver: `test!` checks
//! structural enforcement and invariants, `determinism_test!` checks that two
//! identically-seeded runs agree digest-for-digest, and `leak_test!` checks
//! that one principal's hidden data never reaches another's view.

use crate::entropy::{CtxEntropy, DrawPos, EntropySource, Seed, SeededEntropy};
use crate::rules::{Aggregate, AggregateRules};
use crate::stablehash::{Digest128, StableHash, digest128};
use core::marker::PhantomData;
use ironstate::{EventKind, StateMachine, TransitionRules};
use proptest::strategy::{BoxedStrategy, Strategy, ValueTree};
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};

/// How a machine generates the inputs its test macros drive it with.
pub trait AggregateArbitrary: AggregateRules + Clone + core::fmt::Debug {
    /// A strategy for fresh initial states.
    fn initial_state_strategy() -> BoxedStrategy<Self>;
    /// A strategy for the next command given the current state.
    fn command_strategy(state: &Self) -> BoxedStrategy<Self::Command>;
    /// Build a context over the given entropy for step `step`.
    ///
    /// Takes ownership of the entropy (the contract context owns its stream),
    /// and the context must expose it via `CtxEntropy` so the driver can read
    /// the post-decide position.
    fn test_ctx(entropy: Box<dyn EntropySource>, step: u64) -> Self::Ctx;
}

/// Parameters parsed from a test macro's `key = value` arguments.
#[derive(Debug, Clone)]
pub struct DriveParams {
    /// Number of generated runs.
    pub cases: u32,
    /// Maximum commands per run.
    pub max_steps: usize,
    /// Seed for fully reproducible runs.
    pub seed: u64,
}

impl DriveParams {
    /// Defaults: 256 runs, up to 64 steps, a fixed seed for reproducibility.
    pub fn new() -> Self {
        Self {
            cases: 256,
            max_steps: 64,
            seed: 0x15_0A_7E,
        }
    }
}

impl Default for DriveParams {
    fn default() -> Self {
        Self::new()
    }
}

// --- invariants (optional, found via autoref specialization) --------------

type AggCheck<A> = Box<dyn Fn(&A, &<A as AggregateRules>::Event, &A) -> bool>;

/// A named property checked after every applied event during `test!`.
pub struct AggregateInvariant<A: AggregateRules> {
    description: &'static str,
    check: AggCheck<A>,
}

impl<A: AggregateRules> AggregateInvariant<A> {
    /// Begin defining an invariant.
    pub fn custom(description: &'static str) -> PartialAggregateInvariant<A> {
        PartialAggregateInvariant {
            description,
            _marker: PhantomData,
        }
    }
    /// The description, shown in failure output.
    pub fn description(&self) -> &'static str {
        self.description
    }
    /// Whether the property holds for this `(before, event, after)` step.
    pub fn holds(&self, before: &A, event: &A::Event, after: &A) -> bool {
        (self.check)(before, event, after)
    }
}

/// A half-built [`AggregateInvariant`] awaiting its closure.
pub struct PartialAggregateInvariant<A: AggregateRules> {
    description: &'static str,
    _marker: PhantomData<fn() -> A>,
}

impl<A: AggregateRules> PartialAggregateInvariant<A> {
    /// Supply the property over `(state_before, &event, state_after)`.
    pub fn assert(
        self,
        check: impl Fn(&A, &A::Event, &A) -> bool + 'static,
    ) -> AggregateInvariant<A> {
        AggregateInvariant {
            description: self.description,
            check: Box::new(check),
        }
    }
}

/// Implemented by an aggregate that declares invariants. Optional — a machine
/// without it is still checked structurally.
pub trait AggregateInvariants: AggregateRules {
    /// The invariants to verify after every applied event.
    fn invariants() -> Vec<AggregateInvariant<Self>>;
}

/// Autoref-specialization carrier used by `test!` to find declared invariants
/// at the concrete call site (a generic function could not see the impl).
pub mod invariant_probe {
    use super::{AggregateInvariant, AggregateInvariants};
    use crate::rules::AggregateRules;
    use core::marker::PhantomData;

    /// Dispatch carrier.
    pub struct Probe<A>(pub PhantomData<A>);

    /// Fallback: no declared invariants.
    pub trait ViaNone<A: AggregateRules> {
        /// An empty invariant set.
        fn collect(&self) -> Vec<AggregateInvariant<A>> {
            Vec::new()
        }
    }
    impl<A: AggregateRules> ViaNone<A> for Probe<A> {}

    /// Selected when the aggregate implements `AggregateInvariants`.
    pub trait ViaImpl<A: AggregateRules> {
        /// The declared invariant set.
        fn collect(&self) -> Vec<AggregateInvariant<A>>;
    }
    impl<A: AggregateRules + AggregateInvariants> ViaImpl<A> for &Probe<A> {
        fn collect(&self) -> Vec<AggregateInvariant<A>> {
            <A as AggregateInvariants>::invariants()
        }
    }
}

// --- shared helpers -------------------------------------------------------

fn sample<S: Strategy>(strategy: S, runner: &mut TestRunner) -> S::Value {
    strategy
        .new_tree(runner)
        .expect("strategy failed to produce a value")
        .current()
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

/// Whether `old -> new` is a legal hop of the phase machine (or no change).
fn legal_hop<A: AggregateRules>(old: &A::Phase, new: &A::Phase) -> bool {
    if old == new {
        return true;
    }
    <<A::Phase as TransitionRules>::Event as EventKind>::event_variants()
        .iter()
        .any(|event| old.transition(event).as_ref() == Some(new))
}

// --- test! ----------------------------------------------------------------

/// Drive `test!`: structural enforcement, phase-hop legality, and declared
/// invariants after every applied event.
pub fn run_test<A>(params: DriveParams, invariants: Vec<AggregateInvariant<A>>)
where
    A: AggregateArbitrary,
    A::Ctx: CtxEntropy,
{
    let mut runner = seeded_runner(params.seed);
    for case in 0..params.cases {
        let initial = sample(A::initial_state_strategy(), &mut runner);
        let Ok(mut agg) = Aggregate::new(initial.clone()) else {
            continue;
        };
        let seed = run_seed(params.seed, case);
        let mut pos = DrawPos(0);

        for step in 0..params.max_steps {
            if agg.phase().is_terminal() {
                break;
            }
            let cmd = sample(A::command_strategy(agg.state()), &mut runner);
            let before = agg.state().clone();
            let mut ctx = A::test_ctx(Box::new(SeededEntropy::at(&seed, pos)), step as u64);

            if let Ok(events) = agg.handle(&cmd, &mut ctx) {
                if let Some(entropy) = ctx.entropy_mut() {
                    pos = entropy.draws();
                }
                // Replay the events to check each phase hop and invariant.
                let mut probe = before.clone();
                for event in &events {
                    let phase_before = probe.phase();
                    let state_before = probe.clone();
                    probe.evolve(event);
                    let phase_after = probe.phase();
                    assert!(
                        legal_hop::<A>(&phase_before, &phase_after),
                        "illegal phase hop {phase_before:?} -> {phase_after:?} on event \
                         {event:?}\n  command: {cmd:?}\n  seed: {:#x}, case: {case}",
                        params.seed,
                    );
                    for invariant in &invariants {
                        assert!(
                            invariant.holds(&state_before, event, &probe),
                            "aggregate invariant violated: {}\n  event: {event:?}\n  \
                             before: {state_before:?}\n  after: {probe:?}\n  seed: {:#x}, \
                             case: {case}",
                            invariant.description(),
                            params.seed,
                        );
                    }
                }
            }
        }
    }
}

// --- determinism_test! ----------------------------------------------------

/// Drive `determinism_test!`: two identically-seeded runs must agree on the
/// state digest at every step. A pinned run's final digest is also written to
/// `target/ironstate-determinism/<type>.digest` for the cross-target CI diff.
pub fn run_determinism<A>(params: DriveParams)
where
    A: AggregateArbitrary + StableHash,
    A::Ctx: CtxEntropy,
{
    for case in 0..params.cases {
        let digests_a = one_run::<A>(params.seed, case, params.max_steps);
        let digests_b = one_run::<A>(params.seed, case, params.max_steps);
        assert!(
            digests_a == digests_b,
            "nondeterminism detected: two identically-seeded runs diverged.\n  \
             A: {digests_a:?}\n  B: {digests_b:?}\n  seed: {:#x}, case: {case}\n  \
             A decide/evolve must be a pure function of state, command, and entropy.",
            params.seed,
        );
    }

    // The pinned golden run's final digest, for the cross-target diff.
    if let Some(final_digest) = one_run::<A>(params.seed, 0, params.max_steps)
        .last()
        .copied()
    {
        write_golden(std::any::type_name::<A>(), final_digest);
    }
}

/// Run one full generated run, returning the state digest after every applied
/// step. Two calls with the same seed/case must agree if decide/evolve is pure.
fn one_run<A>(seed: u64, case: u32, max_steps: usize) -> Vec<Digest128>
where
    A: AggregateArbitrary + StableHash,
    A::Ctx: CtxEntropy,
{
    let mut runner = seeded_runner(seed.wrapping_add(u64::from(case)).wrapping_add(1));
    let initial = sample(A::initial_state_strategy(), &mut runner);
    let Ok(mut agg) = Aggregate::new(initial) else {
        return Vec::new();
    };
    let seed_value = run_seed(seed, case);
    let mut pos = DrawPos(0);
    let mut digests = Vec::new();

    for step in 0..max_steps {
        if agg.phase().is_terminal() {
            break;
        }
        let cmd = sample(A::command_strategy(agg.state()), &mut runner);
        let mut ctx = A::test_ctx(Box::new(SeededEntropy::at(&seed_value, pos)), step as u64);
        if agg.handle(&cmd, &mut ctx).is_ok() {
            if let Some(entropy) = ctx.entropy_mut() {
                pos = entropy.draws();
            }
            digests.push(digest128(agg.state()));
        }
    }
    digests
}

fn write_golden(type_name: &str, digest: Digest128) {
    let dir = std::path::Path::new("target/ironstate-determinism");
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let file = type_name.replace("::", "__").replace(['<', '>', ' '], "_");
    let _ = std::fs::write(dir.join(format!("{file}.digest")), digest.to_hex());
}

// --- leak_test! -----------------------------------------------------------

/// An aggregate prepared for leak testing: it can list its principals and
/// produce a sibling state that differs only in one principal's hidden values,
/// keeping their public residue equal.
#[cfg(feature = "redaction")]
pub trait LeakTestable: AggregateArbitrary + crate::redaction::View<Self::Principal> {
    /// The viewing-principal type (the `#[redact(principal = …)]` type).
    type Principal: Clone + PartialEq + core::fmt::Debug;
    /// The principals present in this state.
    fn principals(state: &Self) -> Vec<Self::Principal>;
    /// A copy of `self` with `principal`'s hidden values resampled, keeping
    /// their concealed residue unchanged.
    fn resample_hidden(&self, principal: &Self::Principal, entropy: &mut dyn EntropySource)
    -> Self;
}

/// Drive `leak_test!`: across non-revealing commands, a sibling state that
/// differs only in `p`'s hidden data must produce an identical `view_for(q)` at
/// every step, for every `p != q`.
#[cfg(feature = "redaction")]
pub fn run_leak<A>(params: DriveParams, excluding: &[&str])
where
    A: LeakTestable,
    A::Ctx: CtxEntropy,
    <A as crate::redaction::View<A::Principal>>::Output: PartialEq + core::fmt::Debug,
{
    let mut runner = seeded_runner(params.seed);
    for case in 0..params.cases {
        let base_state = sample(A::initial_state_strategy(), &mut runner);
        let principals = A::principals(&base_state);
        let seed = run_seed(params.seed, case);

        for p in &principals {
            for q in &principals {
                if p == q {
                    continue;
                }
                let mut perturb = SeededEntropy::at(&seed, DrawPos(0));
                let sibling_state = base_state.resample_hidden(p, &mut perturb);

                let (Ok(mut base), Ok(mut sibling)) = (
                    Aggregate::new(base_state.clone()),
                    Aggregate::new(sibling_state),
                ) else {
                    continue;
                };

                // q's view starts equal by construction (only p's hidden differs).
                assert_views_equal::<A>(&base, &sibling, q, p, q, case, params.seed, 0);

                let mut pos = DrawPos(0);
                for step in 0..params.max_steps {
                    if base.phase().is_terminal() {
                        break;
                    }
                    let cmd = sample(A::command_strategy(base.state()), &mut runner);
                    if excluding.contains(&cmd.variant_name()) {
                        continue;
                    }
                    // The same command and entropy position drive both runs.
                    let mut ctx_b =
                        A::test_ctx(Box::new(SeededEntropy::at(&seed, pos)), step as u64);
                    let mut ctx_s =
                        A::test_ctx(Box::new(SeededEntropy::at(&seed, pos)), step as u64);
                    let _ = base.handle(&cmd, &mut ctx_b);
                    let _ = sibling.handle(&cmd, &mut ctx_s);
                    if let Some(entropy) = ctx_b.entropy_mut() {
                        pos = entropy.draws();
                    }
                    assert_views_equal::<A>(&base, &sibling, q, p, q, case, params.seed, step + 1);
                }
            }
        }
    }
}

#[cfg(feature = "redaction")]
#[allow(clippy::too_many_arguments)]
fn assert_views_equal<A>(
    base: &Aggregate<A>,
    sibling: &Aggregate<A>,
    viewer: &A::Principal,
    perturbed: &A::Principal,
    q: &A::Principal,
    case: u32,
    seed: u64,
    step: usize,
) where
    A: LeakTestable,
    A::Ctx: CtxEntropy,
    <A as crate::redaction::View<A::Principal>>::Output: PartialEq + core::fmt::Debug,
{
    use crate::redaction::View;
    let view_base = View::view_for(base, viewer);
    let view_sibling = View::view_for(sibling, viewer);
    assert!(
        view_base == view_sibling,
        "[sampled] leak: principal {q:?}'s view changed when only {perturbed:?}'s hidden \
         data differed, at step {step}.\n  base view:    {view_base:?}\n  sibling view: \
         {view_sibling:?}\n  seed: {seed:#x}, case: {case}",
    );
}
