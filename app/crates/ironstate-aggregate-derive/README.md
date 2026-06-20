# →(Fe) ironstate-aggregate-derive

Derive macros for [ironstate-aggregate](https://docs.rs/ironstate-aggregate):
`StableHash`, `Redact`, and `Versioned`. There is no `Aggregate` derive — the
`AggregateRules` trait is written by hand.

You normally do not depend on this crate directly — depend on
`ironstate-aggregate` (which re-exports these derives) and see its documentation
for usage.

## Learn more

New here? These derives are documented where you actually reach for them, in
[ironstate-aggregate](https://docs.rs/ironstate-aggregate). The
[`hidden-info`](https://github.com/kassian-dev/ironstate/tree/main/app/crates/examples/hidden-info)
example puts `Redact` and `StableHash` to work in a full program, and the
[guide](https://github.com/kassian-dev/ironstate/blob/main/docs/guide.md)
introduces each derive in context.

## The ironstate family

- [`ironstate`](https://crates.io/crates/ironstate) — core lifecycle machines
- [`ironstate-aggregate`](https://crates.io/crates/ironstate-aggregate) — deterministic aggregates (re-exports these derives)
- [`ironstate-journal`](https://crates.io/crates/ironstate-journal) — the event journal

Licensed under Apache-2.0.
