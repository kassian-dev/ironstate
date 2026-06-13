# →(Fe) ironstate-journal

The event journal for [ironstate](https://docs.rs/ironstate) aggregates:
append, snapshot, replay, fork, and subscribe — with the entropy position
recorded atomically beside every event, so an aggregate can be replayed and
resumed bit-identically.

Because replay consumes no entropy (only `decide` draws), the draw position
cannot be recomputed from the events — it must be stored. Every append persists
the post-`decide` position together with its events, in one atomic unit.

```rust,ignore
// The canonical persistent loop: structural checks → decide → append → evolve.
let seq = execute(&mut journal, &mut aggregate, &command, &mut ctx)?;

// Rebuild an aggregate (and its entropy stream) from the journal.
let (aggregate, entropy) = resume(&journal, &seed)?;

// A published, collision-resistant digest anyone can recompute to audit a match.
let digest = replay_hash(snapshot, &events)?;
```

A `Subscription` delivers one aggregate's events to another exactly once
(`(StreamId, Seq)` idempotency keys), and `Versioned` events upcast through a
`MigrateFrom` chain on load.

The reference in-memory journal (feature `memory`, default) passes a
seven-property conformance suite (`journal_contract_test!`) that every storage
adapter is judged against; storage adapters themselves live downstream. Under
feature `sim`, `scenario_test!` drives a seeded whole-tier fault simulation
(append failures, crash/resume, fork/continue) and checks faults are invisible
to outcomes, with the `FaultInjector`/`ReferenceRun` testkit exposed for
consumers' own deterministic-simulation harnesses.

## The ironstate family

- [`ironstate`](https://crates.io/crates/ironstate) — core lifecycle machines
- [`ironstate-aggregate`](https://crates.io/crates/ironstate-aggregate) — deterministic aggregates whose events this journal stores
- [`ironstate-journal`](https://crates.io/crates/ironstate-journal) — the event journal (this crate)

Licensed under Apache-2.0.
