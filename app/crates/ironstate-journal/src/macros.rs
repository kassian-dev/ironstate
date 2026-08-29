//! The journal verification macros.

/// Generate a `#[test]` that runs the journal conformance suite against an
/// adapter for an aggregate.
///
/// One argument tests the reference `MemoryJournal`; two arguments test your own
/// adapter (which must implement `ContractJournal`). Add `forkable` when your
/// adapter implements [`ForkableJournal`](crate::ForkableJournal), which brings
/// in the two extra properties that need branching: round-trip **at each
/// recorded step**, and fork-position equality. Round-trip of the whole log is
/// part of the base contract and runs for every adapter. Add `retainable` when
/// it implements [`RetainableJournal`](crate::RetainableJournal), which adds
/// the truncation-preserves-resume property.
///
/// # Your journal must have `Tx<'_> = ()`
///
/// The suite drives adapters through [`execute`](crate::execute), so it is
/// bound `for<'a> Journal<A, Tx<'a> = ()>`. A journal whose
/// [`Tx`](crate::Journal::Tx) is a real database transaction cannot be passed
/// here as things stand — hold it to the contract with a twin over the same
/// storage whose `Tx` is `()`, the way the `async-store` example does with its
/// synchronous twin.
///
/// ```ignore
/// ironstate_journal::journal_contract_test!(MatchState);                 // the memory journal
/// ironstate_journal::journal_contract_test!(MyPostgresJournal, MatchState);
/// ironstate_journal::journal_contract_test!(MyForkingJournal, MatchState, forkable);
/// ironstate_journal::journal_contract_test!(MyStore, MatchState, forkable, retainable);
/// ```
#[macro_export]
macro_rules! journal_contract_test {
    ($agg:ty) => {
        $crate::journal_contract_test!($crate::MemoryJournal<$agg>, $agg, forkable, retainable);
    };
    ($journal:ty, $agg:ty) => {
        #[test]
        fn journal_contract() {
            $crate::testkit_support::run_contract::<$journal, $agg>(64, 24, 0xC047);
        }
    };
    ($journal:ty, $agg:ty, forkable) => {
        #[test]
        fn journal_contract() {
            $crate::testkit_support::run_contract_forkable::<$journal, $agg>(64, 24, 0xC047);
        }
    };
    ($journal:ty, $agg:ty, retainable) => {
        #[test]
        fn journal_contract() {
            $crate::testkit_support::run_contract::<$journal, $agg>(64, 24, 0xC047);
            $crate::testkit_support::run_contract_retainable::<$journal, $agg>(32, 24, 0xC047);
        }
    };
    ($journal:ty, $agg:ty, forkable, retainable) => {
        #[test]
        fn journal_contract() {
            $crate::testkit_support::run_contract_forkable::<$journal, $agg>(64, 24, 0xC047);
            $crate::testkit_support::run_contract_retainable::<$journal, $agg>(32, 24, 0xC047);
        }
    };
}

/// Generate a `#[test]` for the seeded whole-tier simulation: a fault-injected
/// run must reach the same final digest as a fault-free run over the same
/// commands — faults invisible to outcomes.
///
/// ```ignore
/// ironstate_journal::scenario_test!(MatchState);
/// ironstate_journal::scenario_test!(MatchState, cases = 300, max_steps = 200, seed = 0x51A);
/// ```
#[macro_export]
macro_rules! scenario_test {
    ($agg:ty $(, $key:ident = $val:expr)* $(,)?) => {
        #[test]
        fn scenario() {
            #[allow(unused_mut)]
            let mut cases: u32 = 128;
            #[allow(unused_mut)]
            let mut max_steps: usize = 48;
            #[allow(unused_mut)]
            let mut seed: u64 = 0x5CE_A12;
            $( $crate::__ironstate_scenario_param!(cases, max_steps, seed, $key, $val); )*
            $crate::testkit_support::run_scenario::<$agg>(cases, max_steps, seed);
        }
    };
}

/// Internal: dispatch one `key = value` argument of `scenario_test!`.
#[doc(hidden)]
#[macro_export]
macro_rules! __ironstate_scenario_param {
    ($cases:ident, $max_steps:ident, $seed:ident, cases, $v:expr) => {
        $cases = $v;
    };
    ($cases:ident, $max_steps:ident, $seed:ident, max_steps, $v:expr) => {
        $max_steps = $v;
    };
    ($cases:ident, $max_steps:ident, $seed:ident, seed, $v:expr) => {
        $seed = $v;
    };
}
