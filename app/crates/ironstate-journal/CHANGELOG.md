# Changelog

All notable changes to `ironstate-journal` are recorded here. The format is based
on [Keep a Changelog](https://keepachangelog.com/); from the next release on, the
entries are maintained by release-plz. This crate is the journal tier of the
[ironstate](https://github.com/kassian-dev/ironstate) family.

## [0.3.0](https://github.com/kassian-dev/ironstate/compare/ironstate-journal-v0.2.0...ironstate-journal-v0.3.0) - 2026-08-29

### Added

- `RetainableJournal`: `truncate_before` discards the oldest records in a
  stream, refusing anything that would leave it unresumable; `retained_from`
  reports the horizon and `streams` enumerates what a retention sweep could
  expire. Conformance property 10 proves a truncated stream resumes to a
  bit-identical aggregate.
- `VersionedEvent::new`, the constructor adapters build results with.

### Changed

- **Breaking:** `VersionedEvent` carries the `Seq` of the record an event came
  from, and is now `#[non_exhaustive]`. `events_since` flattens events across
  records, so an index into its result is not a sequence — a subscription keyed
  on one silently dropped events as duplicates once any `decide` emitted more
  than one. Construct through `VersionedEvent::new`; the next field is then
  additive rather than breaking.
- `JournalError` gains `NoBaseForFork` and `NoSnapshotForTruncation`, and
  `UnknownSeq` now also covers reads, forks and truncations below a retained
  horizon.

### Fixed

- `MemoryJournal` no longer panics anywhere: `append_in` and `truncate_before`
  return typed errors rather than asserting internal invariants.
- Truncation no longer strands a fork on a snapshot whose records it discarded,
  which could return a wrong aggregate as `Ok`.
- A fully truncated stream reports its head instead of `None`, so `execute` no
  longer rewinds the entropy stream to `DrawPos(0)`.
- Reading from below the retained horizon is refused rather than returning a
  silently gapped list.

## [0.2.0](https://github.com/kassian-dev/ironstate/compare/ironstate-journal-v0.1.4...ironstate-journal-v0.2.0) - 2026-08-29

Recorded after the fact: 0.2.0 was published by the release workflow on a
version bump rather than through a release-plz PR, so no entry was generated
at the time.

### Changed

- **Breaking:** `Journal` takes a `&StreamId` on every operation, and `Seq` is
  per-stream. One journal value now holds many aggregate instances, which makes
  the write side agree with `Subscription`, whose idempotency key was already
  `(StreamId, Seq)`.
- **Breaking:** `Journal` gains an associated `Tx<'a>`, the caller's unit of
  work, with `append_in`/`snapshot_in`. A journal owning its durability sets it
  to `()`, and `execute`/`resume` keep their previous shape through a
  `for<'a> Journal<A, Tx<'a> = ()>` bound.
- **Breaking:** `fork` moves to `ForkableJournal`. With per-stream sequences a
  whole-journal fork point is ill-defined, and a relational adapter should not
  owe row-copying for a method it never calls.

### Added

- `execute_in` and `Pending`, for a journal enlisting in the caller's
  transaction. `execute_in` deliberately does not evolve the aggregate: a
  rollback after the append would otherwise leave it ahead of the durable log.
- Conformance properties for stream independence and out-of-range addressing;
  `run_contract_forkable` for adapters that branch.

## [0.1.4](https://github.com/kassian-dev/ironstate/compare/ironstate-journal-v0.1.3...ironstate-journal-v0.1.4) - 2026-06-23

### Other

- *(agents)* codify the rustdoc + runnable-examples convention ([#29](https://github.com/kassian-dev/ironstate/pull/29))
- *(quality)* scope mutants to source PRs; document determinism and secret boundaries ([#28](https://github.com/kassian-dev/ironstate/pull/28))

## [0.1.3](https://github.com/kassian-dev/ironstate/compare/ironstate-journal-v0.1.2...ironstate-journal-v0.1.3) - 2026-06-20

### Other

- Additional snippets of examples ([#19](https://github.com/kassian-dev/ironstate/pull/19))

## [0.1.2](https://github.com/kassian-dev/ironstate/compare/ironstate-journal-v0.1.1...ironstate-journal-v0.1.2) - 2026-06-14

### Other

- *(readme)* consistent "Learn more" section across crate READMEs ([#16](https://github.com/kassian-dev/ironstate/pull/16))

## [0.1.1](https://github.com/kassian-dev/ironstate/compare/ironstate-journal-v0.1.0...ironstate-journal-v0.1.1) - 2026-06-14

### Added

- *(journal)* prepare/commit/abort for async adapters, with an async-store example ([#13](https://github.com/kassian-dev/ironstate/pull/13))

### Other

- adopt per-crate changelogs and add a security policy ([#9](https://github.com/kassian-dev/ironstate/pull/9))

## [0.1.0] - 2026-06-13

The initial release.

### Added

- The `Journal` trait with atomic entropy positions and the `MemoryJournal`
  reference implementation.
- `replay`/`resume`/`execute`/`replay_hash`/`fork`.
- The seven-property `journal_contract_test!`.
- `Subscription`/`React` idempotent delivery and the `Versioned` derive.
- The seeded `scenario_test!` with the public `FaultInjector`/`ReferenceRun`
  testkit.
