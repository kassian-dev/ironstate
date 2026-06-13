//! The verification macros: `analyze!` and `test!`.

/// Generate a `#[test]` that runs structural graph analysis on a machine.
///
/// Analysis fails on design errors — unreachable states, deadlocks, inescapable
/// cycles, dead transitions — and prints a labeled report of everything it
/// found. Pass a name to place more than one analysis in the same module.
///
/// ```ignore
/// ironstate::analyze!(NodeState);
/// ironstate::analyze!(NodeState, analyze_node);
/// ```
#[macro_export]
macro_rules! analyze {
    ($ty:ty) => {
        $crate::analyze!($ty, ironstate_analyze);
    };
    ($ty:ty, $name:ident) => {
        #[test]
        fn $name() {
            let report = $crate::analysis_report::analyze::<$ty>();
            eprintln!("{report}");
            report.assert_ok();
        }
    };
}

/// Generate a `#[test]` that runs randomized property testing on a machine.
///
/// After every step it checks structural enforcement and any declared
/// invariants; on failure proptest shrinks to the minimal failing sequence.
///
/// ```ignore
/// ironstate::test!(NodeState);
/// ironstate::test!(NodeState, cases = 1000, max_steps = 50);
/// ironstate::test!(NodeState, seed = 0xDEC0DE);
/// ```
#[macro_export]
macro_rules! test {
    ($ty:ty $(, $key:ident = $val:expr)* $(,)?) => {
        #[test]
        fn ironstate_test() {
            #[allow(unused_mut)]
            let mut params = $crate::testing_support::TestParams::new();
            $( $crate::__ironstate_test_param!(params, $key, $val); )*
            // Collect declared invariants here, where the type is concrete, so
            // autoref specialization can see whether it implements `Invariants`.
            let invariants = {
                #[allow(unused_imports)]
                use $crate::testing_support::probe::{ViaImpl as _, ViaNone as _};
                let probe = $crate::testing_support::probe::Probe::<$ty>(::core::marker::PhantomData);
                (&&probe).collect()
            };
            $crate::testing_support::run::<$ty>(params, invariants);
        }
    };
}

/// Internal: dispatch one `key = value` argument of [`test!`] onto the params.
#[doc(hidden)]
#[macro_export]
macro_rules! __ironstate_test_param {
    ($p:ident, cases, $v:expr) => {
        $p.cases = $v;
    };
    ($p:ident, max_steps, $v:expr) => {
        $p.max_steps = $v;
    };
    ($p:ident, seed, $v:expr) => {
        $p.seed = ::core::option::Option::Some($v);
    };
}
