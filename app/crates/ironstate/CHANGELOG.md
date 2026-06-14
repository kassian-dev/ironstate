# Changelog

All notable changes to `ironstate` are recorded here. The format is based on
[Keep a Changelog](https://keepachangelog.com/); from the next release on, the
entries are maintained by release-plz. This crate is the lifecycle-machine core
of the [ironstate](https://github.com/kassian-dev/ironstate) family.

## [0.1.1](https://github.com/kassian-dev/ironstate/compare/ironstate-v0.1.0...ironstate-v0.1.1) - 2026-06-14

### Other

- adopt per-crate changelogs and add a security policy ([#9](https://github.com/kassian-dev/ironstate/pull/9))

## [0.1.0] - 2026-06-13

The initial release.

### Added

- `#[derive(StateMachine)]` / `#[derive(Event)]` and the `Machine` runtime with
  `apply`/`could_apply`/`why_not`/`peek_transition`.
- Structural enforcement (terminal states, event-kind restrictions) and typed
  `TransitionError`.
- Sync listeners with an injectable clock, and declared `Invariants`.
- Versioned restore (`version`/`history`/`MigrateFrom`) behind the default-on
  `restore` feature.
- The `analyze!` / `test!` verification macros with `[proven]`/`[sampled]` labels.
