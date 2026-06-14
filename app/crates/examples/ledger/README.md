# ledger

An account ledger modeled as an ironstate aggregate over a journal.

It shows the aggregate + journal tier end to end without redaction or
entropy: a struct state changed by `decide`/`evolve`, a non-negative-balance
invariant, a memory journal driven by `execute`, and `resume` reproducing
the balance from the event log. Money is integer cents — no floats.

## Run it

```sh
# cargo commands run from app/
cargo run -p ledger    # the demo
cargo test -p ledger   # the end-to-end test module
```
