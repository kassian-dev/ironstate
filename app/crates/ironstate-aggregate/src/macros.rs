//! The aggregate verification macros: `test!`, `determinism_test!`,
//! `leak_test!`.

/// Generate a `#[test]` that property-tests an aggregate: structural
/// enforcement, phase-hop legality, and declared invariants after every applied
/// event.
///
/// ```ignore
/// ironstate_aggregate::test!(MatchState, cases = 1000, max_steps = 80);
/// ```
#[macro_export]
macro_rules! test {
    ($ty:ty $(, $key:ident = $val:expr)* $(,)?) => {
        #[test]
        fn ironstate_aggregate_test() {
            #[allow(unused_mut)]
            let mut params = $crate::testkit_support::DriveParams::new();
            $( $crate::__ironstate_aggregate_param!(params, $key, $val); )*
            // Collect declared invariants where the type is concrete.
            let invariants = {
                #[allow(unused_imports)]
                use $crate::testkit_support::invariant_probe::{ViaImpl as _, ViaNone as _};
                let probe = $crate::testkit_support::invariant_probe::Probe::<$ty>(
                    ::core::marker::PhantomData,
                );
                (&&probe).collect()
            };
            $crate::testkit_support::run_test::<$ty>(params, invariants);
        }
    };
}

/// Generate a `#[test]` that checks two identically-seeded runs agree
/// digest-for-digest — catching nondeterminism in `decide`/`evolve`.
///
/// ```ignore
/// ironstate_aggregate::determinism_test!(MatchState);
/// ```
#[macro_export]
macro_rules! determinism_test {
    ($ty:ty $(, $key:ident = $val:expr)* $(,)?) => {
        #[test]
        fn ironstate_aggregate_determinism_test() {
            #[allow(unused_mut)]
            let mut params = $crate::testkit_support::DriveParams::new();
            $( $crate::__ironstate_aggregate_param!(params, $key, $val); )*
            $crate::testkit_support::run_determinism::<$ty>(params);
        }
    };
}

/// Generate a `#[test]` that checks one principal's hidden data never reaches
/// another's view, across non-revealing commands.
///
/// `excluding` lists the command variants that legitimately reveal hidden
/// information (so they are skipped); the test is `[sampled]`.
///
/// ```ignore
/// ironstate_aggregate::leak_test!(MatchState, cases = 2000, excluding = [PlayCard, RevealStray]);
/// ```
#[macro_export]
macro_rules! leak_test {
    ($ty:ty $(, $key:ident = $val:tt)* $(,)?) => {
        #[test]
        fn ironstate_aggregate_leak_test() {
            #[allow(unused_mut)]
            let mut params = $crate::testkit_support::DriveParams::new();
            #[allow(unused_mut)]
            let mut excluding: ::std::vec::Vec<&'static str> = ::std::vec::Vec::new();
            $( $crate::__ironstate_aggregate_leak_param!(params, excluding, $key, $val); )*
            $crate::testkit_support::run_leak::<$ty>(params, &excluding);
        }
    };
}

/// Internal: dispatch one `key = value` argument of `test!`/`determinism_test!`.
#[doc(hidden)]
#[macro_export]
macro_rules! __ironstate_aggregate_param {
    ($p:ident, cases, $v:expr) => {
        $p.cases = $v;
    };
    ($p:ident, max_steps, $v:expr) => {
        $p.max_steps = $v;
    };
    ($p:ident, seed, $v:expr) => {
        $p.seed = $v;
    };
}

/// Internal: dispatch one argument of `leak_test!`, including `excluding = [..]`.
#[doc(hidden)]
#[macro_export]
macro_rules! __ironstate_aggregate_leak_param {
    ($p:ident, $ex:ident, cases, $v:tt) => {
        $p.cases = $v;
    };
    ($p:ident, $ex:ident, max_steps, $v:tt) => {
        $p.max_steps = $v;
    };
    ($p:ident, $ex:ident, seed, $v:tt) => {
        $p.seed = $v;
    };
    ($p:ident, $ex:ident, excluding, [ $($cmd:ident),* $(,)? ]) => {
        $( $ex.push(::core::stringify!($cmd)); )*
    };
}
