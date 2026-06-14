# Changelog

All notable changes to `ironstate-journal` are recorded here. The format is based
on [Keep a Changelog](https://keepachangelog.com/); from the next release on, the
entries are maintained by release-plz. This crate is the journal tier of the
[ironstate](https://github.com/kassian-dev/ironstate) family.

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
