# Changelog

All notable changes to `ironstate-journal` are recorded here. The format is based
on [Keep a Changelog](https://keepachangelog.com/); from the next release on, the
entries are maintained by release-plz. This crate is the journal tier of the
[ironstate](https://github.com/kassian-dev/ironstate) family.

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
