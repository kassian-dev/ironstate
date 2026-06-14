# ironstate examples

Each subdirectory is a runnable crate whose test module is an end-to-end test of
a realistic use case. They are `publish = false` — they exist to be read, run,
and to keep the family honest (each builds and tests in CI).

Not sure where to start? Pick by what you're building:

| Example | Tier | What it shows | Test layers | Start here if… |
|---------|------|---------------|-------------|----------------|
| [`release-pipeline`](release-pipeline) | core machine | operator/external-gated states, declared invariants, a pure transition function | `analyze!`, `test!` | you want a plain state machine: states, legal moves, structural checks |
| [`ledger`](ledger) | aggregate + journal | `decide`/`evolve`, a non-negative-balance invariant, `execute`/`resume` | `test!`, `determinism_test!`, `journal_contract_test!` | your state changes through a stream of events you want to replay |
| [`hidden-info`](hidden-info) | full stack | redaction (`#[hidden]`), journaled entropy, a subscription — the integration template | all five: `test!`, `determinism_test!`, `leak_test!`, `journal_contract_test!`, `scenario_test!` | you have per-viewer secrets (a card game, sealed bids) |
| [`catalog-ctx`](catalog-ctx) | aggregate (adoption) | porting a *borrowing* turn context to the owned `Ctx` (catalog by `Arc`, entropy by `Box`) | `test!`, `determinism_test!`, `journal_contract_test!` | you're adopting ironstate in an engine that threads a `&mut` context |
| [`async-store`](async-store) | journal adapter (adoption) | making an async store durable via `prepare`/`commit`/`abort`, kept under contract by a sync twin | `journal_contract_test!` | your source of truth is an async database (tokio-postgres, etc.) |

The first three mirror the [guide](../../../docs/guide.md); the last two are
adoption recipes for real integrations. For what each test layer proves and when
to reach for it, see [docs/testing.md](../../../docs/testing.md).

## Running them

```sh
# cargo commands run from app/
cargo run -p ledger      # run an example
cargo test -p ledger     # its end-to-end test module
make test                # build and test every example (from the repo root)
```
