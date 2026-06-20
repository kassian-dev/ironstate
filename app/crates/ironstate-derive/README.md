# →(Fe) ironstate-derive

Derive macros for [ironstate](https://docs.rs/ironstate): `#[derive(StateMachine)]`
and `#[derive(Event)]`.

You normally do not depend on this crate directly — depend on `ironstate` (which
re-exports these derives behind its default `derive` feature) and see its
documentation for usage.

## Learn more

New here? These derives are documented where you actually reach for them, in
[ironstate](https://docs.rs/ironstate). The [guide](https://github.com/kassian-dev/ironstate/blob/main/docs/guide.md)
builds a machine with `#[derive(StateMachine)]` and `#[derive(Event)]` from the
first line, and the [`release-pipeline`](https://github.com/kassian-dev/ironstate/tree/main/app/crates/examples/release-pipeline)
example is a complete program that uses both.

## The ironstate family

- [`ironstate`](https://crates.io/crates/ironstate) — core lifecycle machines (re-exports these derives)
- [`ironstate-aggregate`](https://crates.io/crates/ironstate-aggregate) — deterministic aggregates
- [`ironstate-journal`](https://crates.io/crates/ironstate-journal) — the event journal

Licensed under Apache-2.0.
