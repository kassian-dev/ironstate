# Changelog

All notable changes to the ironstate family are recorded here. The format is
based on [Keep a Changelog](https://keepachangelog.com/); the crates version
together during 0.x. Future entries are maintained by release-plz.

## [0.1.0] - 2026-06-13

The initial release of the crate family (superseding the `ironstate 0.0.1`
placeholder).

### Added

- **`ironstate`** (core) — `#[derive(StateMachine)]` / `#[derive(Event)]`, the
  `Machine` runtime with `apply`/`could_apply`/`why_not`/`peek_transition`,
  structural enforcement (terminal states, event-kind restrictions), typed
  `TransitionError`, sync listeners with an injectable clock, declared
  `Invariants`, versioned restore (`version`/`history`/`MigrateFrom`), and the
  `analyze!` / `test!` verification macros with `[proven]`/`[sampled]` labels.
- **`ironstate-aggregate`** — the `decide`/`evolve` aggregate runtime
  (`AggregateRules`, `Aggregate`, `Rejection`, `why_not`), the counter-addressable
  `SeededEntropy` and `EntropySource` API, redaction (`Conceal`/`OwnerRedact`/
  `PerPrincipal`/`Owned`/`View` + the `Redact` derive), the frozen-encoding
  `StableHash` (`Digest128` + BLAKE3 `AuditDigest`) with teaching compile-errors,
  and the `test!` / `determinism_test!` / `leak_test!` macros.
- **`ironstate-journal`** — the `Journal` trait with atomic entropy positions,
  the `MemoryJournal` reference implementation, `replay`/`resume`/`execute`/
  `replay_hash`/`fork`, the seven-property `journal_contract_test!`,
  `Subscription`/`React` idempotent delivery, the `Versioned` derive, and the
  seeded `scenario_test!` with the public `FaultInjector`/`ReferenceRun` testkit.
- **Examples** (end-to-end tests): `hidden-info`, `release-pipeline`, `ledger`.

### Notes

- The `ironstate-lints` dylint crate was considered and dropped; the determinism
  contract it would enforce is already executable via the `StableHash`
  compile-errors, the clock-free entropy API, and `determinism_test!`. See
  `docs/decisions/0002-deferred-and-out-of-scope.md`.
