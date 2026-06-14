# async-store

Making an **async, authoritative store** durable through ironstate without
implementing the synchronous `Journal` trait.

This is the recollect shape: the source of truth is a database reached over an
async client (tokio-postgres, say). Its `append`/read operations are `async fn`,
so it cannot implement ironstate's synchronous `Journal` — that trait's
`fn append(&mut self, …) -> Result<Seq, JournalError>` has no place to `.await`.
The reflex fixes are unappealing: a blocking client behind `spawn_blocking`
(a second, sync connection pool beside the async one), or a `block_on` bridge
that re-enters the runtime (an anti-pattern).

The clean path keeps the async stack and still gets the durability guarantee,
because the load-bearing discipline of the persistent loop is not in the
`Journal` trait — it is in three pure steps ironstate exposes:

- `prepare` — structural checks, `decide`, and the entropy-position capture,
  with rewind-on-rejection. Pure; touches no storage.
- `Prepared::commit` — evolve the aggregate after the append succeeds.
- `Prepared::abort` — rewind the entropy stream after the append fails.

A consumer owns only the IO: read the head position, `prepare`, **`.await` its
own append**, then `commit` or `abort`. There is exactly one mutating `.await`
and no entropy/ordering logic to copy, so the async loop cannot drift from the
built-in `execute`. `resume` has the same shape: this example's async resume
reads the store with `.await` and feeds the pure `replay` primitive.

**Keeping the durable path under the contract.** The catch with rolling your own
loop is that the storage — the part that can actually corrupt durability — sits
outside `journal_contract_test!`. The fix is a *synchronous twin*: a `Journal`
over the very same storage (here `SyncStore` over `Log`) that exists only to
be measured by the seven-property suite. Production drives the storage through
the async front end (`AsyncStore`); the contract proves the storage semantics
through the sync one. Same `Log`, two front ends — so "we went async" never means
"we left the yardstick behind."

## Run it

```sh
# cargo commands run from app/
cargo run -p async-store    # the demo
cargo test -p async-store   # the end-to-end test module
```
