# Changelog

The ironstate family keeps a changelog per crate, each maintained by release-plz
on release. This file is the family index and records family-wide milestones; see
each crate's changelog for its detailed notes.

- [`ironstate`](app/crates/ironstate/CHANGELOG.md) — lifecycle-machine core
- [`ironstate-derive`](app/crates/ironstate-derive/CHANGELOG.md) — core derives
- [`ironstate-aggregate`](app/crates/ironstate-aggregate/CHANGELOG.md) — aggregate tier
- [`ironstate-aggregate-derive`](app/crates/ironstate-aggregate-derive/CHANGELOG.md) — aggregate derives
- [`ironstate-journal`](app/crates/ironstate-journal/CHANGELOG.md) — journal tier

## 0.1.0 - 2026-06-13

The initial public release of the family (superseding the `ironstate 0.0.1`
placeholder). All five crates released together at 0.1.0; see each crate's
changelog above for its notes.

- The `ironstate-lints` dylint crate was considered and dropped; the determinism
  contract it would enforce is already executable via the `StableHash`
  compile-errors, the clock-free entropy API, and `determinism_test!`. See
  [`docs/decisions/0002-deferred-and-out-of-scope.md`](docs/decisions/0002-deferred-and-out-of-scope.md).
