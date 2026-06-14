# Changelog

All notable changes to `ironstate-aggregate` are recorded here. The format is
based on [Keep a Changelog](https://keepachangelog.com/); from the next release
on, the entries are maintained by release-plz. This crate is the aggregate tier
of the [ironstate](https://github.com/kassian-dev/ironstate) family.

## [0.1.2](https://github.com/kassian-dev/ironstate/compare/ironstate-aggregate-v0.1.1...ironstate-aggregate-v0.1.2) - 2026-06-14

### Other

- *(readme)* consistent "Learn more" section across crate READMEs ([#16](https://github.com/kassian-dev/ironstate/pull/16))

## [0.1.1](https://github.com/kassian-dev/ironstate/compare/ironstate-aggregate-v0.1.0...ironstate-aggregate-v0.1.1) - 2026-06-14

### Other

- adopt per-crate changelogs and add a security policy ([#9](https://github.com/kassian-dev/ironstate/pull/9))

## [0.1.0] - 2026-06-13

The initial release.

### Added

- The `decide`/`evolve` aggregate runtime (`AggregateRules`, `Aggregate`,
  `Rejection`, `why_not`).
- The counter-addressable `SeededEntropy` and the `EntropySource` API.
- Redaction (`Conceal`/`OwnerRedact`/`PerPrincipal`/`Owned`/`View` + the `Redact`
  derive).
- The frozen-encoding `StableHash` (`Digest128` + BLAKE3 `AuditDigest`) with
  teaching compile-errors.
- The `test!` / `determinism_test!` / `leak_test!` macros.
