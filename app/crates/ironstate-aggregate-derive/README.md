# ironstate-aggregate-derive

Derive macros for [ironstate-aggregate](https://docs.rs/ironstate-aggregate):
`StableHash`, `Redact`, and `Versioned`. There is no `Aggregate` derive — the
`AggregateRules` trait is written by hand.

You normally do not depend on this crate directly — depend on
`ironstate-aggregate` (which re-exports these derives) and see its documentation
for usage.

Licensed under Apache-2.0.
