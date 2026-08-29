# outbox

A journal that enlists in the **caller's** transaction, so an append and the
work it causes land together or not at all.

An order ships. Three things must become true at once:

- the `Shipped` event is in the log,
- the order's row in the read model says `shipped`,
- a "send the shipment confirmation" job is on the queue.

If the append commits and the job does not, the customer is never told their
order shipped, and nothing in the system knows. That is not a bug you can
retry your way out of after the fact — the information that it needed doing is
gone. So all three go into one transaction, which is why this example exists
and why an external queue is not used.

## The shape

`Journal::Tx<'a>` is whatever unit of work the caller already has.
`MemoryJournal` owns its durability and sets it to `()`. A journal living in
the same database as your read models sets it to that database's transaction:

```rust,ignore
let mut tx = store.begin();
let pending = execute_in(&mut journal, &mut tx, &stream, &order, &cmd, &mut ctx)?;
tx.upsert_read_model(id, OrderRow::shipped());   // the caller's own writes,
tx.enqueue(Job::ShipmentConfirmation { id });    // in the same transaction
store.commit(tx);
let seq = pending.commit(&mut order);
```

`Store` here is a few dozen lines of in-memory tables standing in for SQLite or
Postgres. The point is not the storage; it is that `Transaction` is the
caller's type, the journal writes into it rather than around it, and nothing is
visible until `commit`.

## Why `execute_in` does not evolve the aggregate

[`execute`](https://docs.rs/ironstate-journal) applies the events as soon as
the append returns, which is right when the append *is* the commit. Under an
enclosing transaction it is wrong: a rollback afterwards would leave the
in-memory aggregate ahead of the durable log, silently, with no error anywhere
— the exact failure the seam exists to prevent, reintroduced one layer up.

So `execute_in` hands back a `Pending` instead. You drive it once your
transaction resolves: `commit` to advance the aggregate, `abort` to rewind the
entropy stream. It is `#[must_use]`, so forgetting is a warning rather than a
silent divergence.

## One call per transaction

`Journal::head` and `entropy_pos` take no `tx`, so they see committed state
only. A second `execute_in` against the same open transaction would compute
its rewind anchor from the pre-transaction head, and aborting it would rewind
past the first append's draws. Batching several commands atomically needs a
journal whose reads are transaction-aware, which the trait does not yet
express — so: one `execute_in` per unit of work.

## Run it

```sh
# cargo commands run from app/
cargo run -p outbox    # the demo: one commit, one rollback
cargo test -p outbox   # the atomicity tests
```

The demo ships one order and then fails the transaction on another, printing
the store after each so you can see that a rolled-back append leaves no event,
no read-model row, no queued job, and an un-evolved aggregate.
