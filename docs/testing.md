# Testing taxonomy

Each layer below states what it proves and where it lives. The done-gate is
`make check`; the heavier suites run in CI on their own cadence.

## Layers

| Layer | Proves | Location |
|-------|--------|----------|
| Unit | One rule per test, named for the rule | `#[cfg(test)]` next to the code |
| Integration | The public API and the verification macros end to end | `crates/<crate>/tests/` |
| Structural analysis | `[proven]` graph facts: reachability, deadlocks, dead transitions | `analyze!()` |
| Property | Invariants hold over randomized sequences | `test!()` |
| Determinism | Same seed + inputs ⇒ identical digest, byte-for-byte across targets | `determinism_test!()`; CI diffs the digests on x86_64 vs aarch64 |
| Leak | No covert flow from one principal's hidden state to another's view | `leak_test!()` |
| Conformance | A journal adapter satisfies the seven contract properties | `journal_contract_test!()` |
| Entropy contract | A custom `EntropySource` draws in range and covers it, reconstructs its stream on seek, and forks purely | `assert_entropy_contract()` |
| Simulation | Faults are invisible to outcomes | `scenario_test!()` |
| Fuzz / hostile input | Garbage is rejected with a typed error, never a panic | `cargo-fuzz` on the versioned-restore decode path; blocks on a crash (uploads the reproducer), every PR |
| Mutation | The suite actually catches bugs | `cargo-mutants --in-diff` on changed code; advisory (run summary, never blocks), manual via `mutants.yml` (workflow_dispatch) |
| Compile-fail | Derive diagnostics teach (exact stderr) | `trybuild` fixtures |

## When to reach for each (the macros you write)

Most of the table above is machinery the family runs for you — fuzz, mutation,
and compile-fail are CI/maintainer layers — or that every machine gets for free
(`analyze!`). The macros you *decide* to add depend on what your type does:

| Macro | Add it when… | Skip it when… |
|-------|--------------|---------------|
| `analyze!` | always, for any core `StateMachine` — free, no runtime, and it catches design errors (unreachable states, deadlocks, dead transitions) | — |
| `test!` | your machine or aggregate has invariants or non-trivial rules worth exercising over random sequences | a trivial toggle with nothing to assert |
| `determinism_test!` | your aggregate **draws entropy**, or you rely on replay / audit digests / cross-target reproducibility | no entropy and the state is trivially deterministic |
| `leak_test!` | you have `#[hidden]` fields or per-viewer redaction (hidden-information domains) | nothing is concealed — there's nothing to leak |
| `journal_contract_test!` | you **implement your own storage adapter** — it proves the seven durability properties | you use `MemoryJournal` or a provided adapter; it's already covered |
| `assert_entropy_contract()` | you **implement your own `EntropySource`**, or override its derived draws — it proves the in-range/covering-draw, seek-reconstruction, and pure-probe contract the determinism guarantee rests on | you use the provided `SeededEntropy` |
| `scenario_test!` | durability under faults matters and you want crashes / forks / retries proven invisible to the outcome | an in-memory or non-durable use case |

The two most often misapplied: `journal_contract_test!` is for *adapter authors*,
not every consumer, and `leak_test!` only applies once something is `#[hidden]`.
The [examples](../app/crates/examples) show the common combinations.

## Rules

- **Red tests are executable specifications.** Unbuilt features live as
  `#[ignore = "reason"]` tests; the ignore reason is the contract. They get
  implemented, never deleted.
- **A bug fix ships with the test that would have caught it.**
- **No flaky tests.** Anything nondeterministic takes a seed.
- **Golden vectors are write-once.** Entropy streams and digests are generated
  with the code under test in the same change, and never regenerated to turn a
  red test green — doing so silently destroys the cross-platform determinism
  guarantee the family exists to provide. A vector changes only when the
  contract intentionally changes, reviewed as a contract change.
- **Test-the-testers fixtures.** Planted defects (a leaky aggregate, a
  nondeterministic one, a snapshot-position confusion, an entropy source with an
  out-of-range or degenerate draw, a broken or forward-only seek, a drifting
  probe, or a bad shuffle) must be *caught* by their macro or check; a build
  where a planted defect goes uncaught is red.

## What is implemented and how it is covered

The family is built. Every crate is tested under `cargo test --workspace
--all-features`, with `cargo fmt --check` and `cargo clippy -D warnings` clean
and the rustdoc building under `-D warnings`.

- **Core (`ironstate`)** — structural enforcement, the
  `apply`/`could_apply`/`why_not`/`peek_transition` quartet, typed
  `TransitionError`, event kinds, versioned restore through a `MigrateFrom`
  chain, and the `analyze!`/`test!` macros with `[proven]`/`[sampled]` labels.
- **StableHash (`ironstate-aggregate`)** — done: the frozen canonical encoding,
  `Digest128` (rustc-stable-hash) and `AuditDigest` (BLAKE3), the `StableHash`
  derive with teaching compile-errors for floats / hash maps / wall clocks and
  `#[stable_hash(skip)]`. Covered by injectivity tests, PartialEq conformance,
  committed golden vectors (write-once), and `trybuild` compile-fail fixtures.
  Also builds on wasm32 to prove the encoding carries no host coupling, while CI
  diffs the seeded digests across x86_64 and aarch64 to confirm they match.

- **Aggregate runtime (`ironstate-aggregate`)** — done: `AggregateRules`,
  `Aggregate<A>` with the `handle` algorithm (terminal → kind → decide →
  evolve), `Rejection`/`InitError`, `why_not`, and the counter-addressable
  `SeededEntropy` (ChaCha12 with O(1) seek, never `Clone`) behind the
  `EntropySource` trait, with `DeterministicCtx`/`OwnedDeterministicCtx`.
  Covered by the decide/evolve-equivalence law, structural-enforcement and
  init checks, the reusable `assert_entropy_contract` conformance check (proven
  on `SeededEntropy`, with planted-defect fixtures the check must catch), and
  frozen golden stream vectors.

- **Redaction + aggregate test macros (`ironstate-aggregate`)** — done:
  `Conceal`/`OwnerRedact`/`PerPrincipal`/`Owned`/`Projection`, the `Redact`
  derive (per-viewer `XView` + `view_for`, with a teaching `on_unimplemented`
  diagnostic), and the `AggregateArbitrary`/`AggregateInvariants`/`LeakTestable`
  traits behind `test!`/`determinism_test!`/`leak_test!`. The test-the-testers
  fixtures pass: `determinism_test!` catches HashMap-iteration-in-`decide`, and
  `leak_test!` catches a hidden value flowing into a public field (while passing
  a clean aggregate).

- **Journal (`ironstate-journal`)** — done: the `Journal` trait (atomic
  `append` of events + entropy position), `Seq`/`Snapshot`/`VersionedEvent`/
  `JournalError`/`ExecuteError`, the `MemoryJournal` reference implementation,
  and `replay`/`resume`/`execute`/`replay_hash`/`fork`. The
  `journal_contract_test!` seven-property conformance suite passes against the
  memory journal (round-trip, position totality & monotonicity, resume identity,
  fork-position equality, snapshot-vs-head discipline, failed-append atomicity,
  version tagging). `Subscription`/`React` deliver idempotently (duplicates and
  out-of-order redeliveries dropped, converging to exactly-once). The
  `Versioned` derive gives events/snapshots a `MigrateFrom` chain.

- **Seeded simulation (`ironstate-journal`, feature `sim`)** — done: the public
  testkit (`Fault`, `FaultSchedule`, `FaultInjector`, `ReferenceRun`) and
  `scenario_test!`. One seed drives a command stream plus an interleaved fault
  schedule (append-failure-and-retry, crash-and-resume, fork-and-continue); the
  faulted run must reach the same final `Digest128` as the fault-free reference
  run — faults invisible to outcomes.

- **Examples as end-to-end tests** — done: `hidden-info` (redaction + journal +
  subscription, all five macros), `release-pipeline` (a core lifecycle machine),
  `ledger` (an aggregate over a journal), `catalog-ctx` (the owned `Ctx`: a
  catalog plus a live entropy stream), and `async-store` (an async adapter kept
  under `journal_contract_test!` by a sync twin). Each example's test module is an
  e2e test of a realistic use case; the [examples index](../app/crates/examples)
  maps each to the tier and test layers it exercises.

The proptest `subscription_test!` over generated redelivery remains an
enhancement on top of the tested in-process `Subscription`.
