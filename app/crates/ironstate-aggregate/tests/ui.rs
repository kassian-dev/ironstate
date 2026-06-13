//! Compile-fail fixtures: the StableHash derive must reject types that cannot be
//! deterministically hashed, with a teaching message naming the fix. The paired
//! `.stderr` files pin the exact diagnostics.

#[test]
fn stable_hash_teaching_errors() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
