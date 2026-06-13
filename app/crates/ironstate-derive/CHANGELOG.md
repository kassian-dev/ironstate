# Changelog

All notable changes to `ironstate-derive` are recorded here. The format is based
on [Keep a Changelog](https://keepachangelog.com/); from the next release on, the
entries are maintained by release-plz. This crate provides the procedural macros
for [`ironstate`](https://github.com/kassian-dev/ironstate).

## [0.1.0] - 2026-06-13

The initial release.

### Added

- `#[derive(StateMachine)]` and `#[derive(Event)]`.
- The versioned-restore derive (`version`/`history`/`MigrateFrom`).
