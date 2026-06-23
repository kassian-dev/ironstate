# Changelog

All notable changes to `ironstate-aggregate-derive` are recorded here. The format
is based on [Keep a Changelog](https://keepachangelog.com/); from the next release
on, the entries are maintained by release-plz. This crate provides the procedural
macros for [`ironstate-aggregate`](https://github.com/kassian-dev/ironstate).

## [0.1.3](https://github.com/kassian-dev/ironstate/compare/ironstate-aggregate-derive-v0.1.2...ironstate-aggregate-derive-v0.1.3) - 2026-06-23

### Other

- *(agents)* codify the rustdoc + runnable-examples convention ([#29](https://github.com/kassian-dev/ironstate/pull/29))
- *(quality)* scope mutants to source PRs; document determinism and secret boundaries ([#28](https://github.com/kassian-dev/ironstate/pull/28))

## [0.1.2](https://github.com/kassian-dev/ironstate/compare/ironstate-aggregate-derive-v0.1.1...ironstate-aggregate-derive-v0.1.2) - 2026-06-20

### Other

- Additional snippets of examples ([#19](https://github.com/kassian-dev/ironstate/pull/19))

## [0.1.1](https://github.com/kassian-dev/ironstate/compare/ironstate-aggregate-derive-v0.1.0...ironstate-aggregate-derive-v0.1.1) - 2026-06-14

### Other

- adopt per-crate changelogs and add a security policy ([#9](https://github.com/kassian-dev/ironstate/pull/9))

## [0.1.0] - 2026-06-13

The initial release.

### Added

- `#[derive(StableHash)]` and `#[derive(Redact)]`.
