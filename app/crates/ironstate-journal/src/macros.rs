//! The journal verification macros.

/// Generate a `#[test]` that runs the seven-property conformance suite against a
/// journal adapter for an aggregate.
///
/// One argument tests the reference `MemoryJournal`; two arguments test your own
/// adapter (which must implement `ContractJournal`).
///
/// ```ignore
/// ironstate_journal::journal_contract_test!(MatchState);                 // the memory journal
/// ironstate_journal::journal_contract_test!(MyPostgresJournal, MatchState);
/// ```
#[macro_export]
macro_rules! journal_contract_test {
    ($agg:ty) => {
        $crate::journal_contract_test!($crate::MemoryJournal<$agg>, $agg);
    };
    ($journal:ty, $agg:ty) => {
        #[test]
        fn journal_contract() {
            $crate::testkit_support::run_contract::<$journal, $agg>(64, 24, 0xC047);
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
