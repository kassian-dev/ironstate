# →(Fe) ironstate-journal

The event journal for [ironstate](https://docs.rs/ironstate) aggregates:
append, snapshot, replay and subscribe — with the entropy position recorded
atomically beside every event, so an aggregate can be replayed and resumed
bit-identically.

Because replay consumes no entropy (only `decide` draws), the draw position
cannot be recomputed from the events — it must be stored. Every append persists
the post-`decide` position together with its events, in one atomic unit.

One journal holds **many streams**, each an aggregate instance with its own
`Seq` line, so the write side addresses history the same way `Subscription`
always has.

```rust,ignore
// The canonical persistent loop: structural checks → decide → append → evolve.
let stream = StreamId::new("account-42");
let seq = execute(&mut journal, &stream, &mut aggregate, &command, &mut ctx)?;

// Rebuild an aggregate (and its entropy stream) from the journal.
let (aggregate, entropy) = resume(&journal, &stream, &seed)?;

// A published, collision-resistant digest anyone can recompute to audit a match.
let digest = replay_hash(snapshot, &events)?;
```

## Appending inside your own transaction

A journal that owns its durability sets `type Tx<'a> = ()` and the calls above
apply unchanged. A journal living in the same database as your read models sets
`Tx` to that database's transaction, so the append and your other writes commit
together or not at all:

```rust,ignore
let mut tx = pool.begin()?;
let pending = execute_in(&mut journal, &mut tx, &stream, &aggregate, &cmd, &mut ctx)?;
write_read_model(&mut tx, ...)?;        // your other writes,
enqueue_outbound_job(&mut tx, ...)?;    // in the same transaction
match tx.commit() {
    Ok(()) => { let seq = pending.commit(&mut aggregate); }
    Err(e) => { pending.abort(&mut ctx); return Err(e.into()); }
}
```

`execute_in` deliberately does *not* evolve the aggregate — a rollback after the
append would otherwise leave it silently ahead of the durable log. The returned
`Pending` is driven once your transaction resolves.

Two capabilities are opt-in, because not every store can or should offer them:
`ForkableJournal` branches a stream's history (essential for simulation,
meaningless for a statutory record), and `RetainableJournal` truncates the
oldest records once a snapshot covers what remains.

A `Subscription` delivers one aggregate's events to another exactly once
(`(StreamId, Seq)` idempotency keys), and `Versioned` events upcast through a
`MigrateFrom` chain on load.

The reference in-memory journal (feature `memory`, default) passes the
conformance suite (`journal_contract_test!`) that every storage adapter is
judged against — eight properties for any journal, plus two for forkable ones
and one for retainable ones; storage adapters themselves live downstream. Under
feature `sim`, `scenario_test!` drives a seeded whole-tier fault simulation
(append failures, crash/resume, fork/continue) and checks faults are invisible
to outcomes, with the `FaultInjector`/`ReferenceRun` testkit exposed for
consumers' own deterministic-simulation harnesses.

## Learn more

New here? The [guide](https://github.com/kassian-dev/ironstate/blob/main/docs/guide.md) is a step-by-step walkthrough from a first machine through aggregates and the event journal. For complete runnable programs, see the [`ledger`](https://github.com/kassian-dev/ironstate/tree/main/app/crates/examples/ledger) example (`execute`/`resume` over a journal) and [`async-store`](https://github.com/kassian-dev/ironstate/tree/main/app/crates/examples/async-store) (a real async storage adapter kept under the conformance suite). For what each test layer proves, see [docs/testing.md](https://github.com/kassian-dev/ironstate/blob/main/docs/testing.md).

## The ironstate family

- [`ironstate`](https://crates.io/crates/ironstate) — core lifecycle machines
- [`ironstate-aggregate`](https://crates.io/crates/ironstate-aggregate) — deterministic aggregates whose events this journal stores
- [`ironstate-journal`](https://crates.io/crates/ironstate-journal) — the event journal (this crate)

Licensed under Apache-2.0.
