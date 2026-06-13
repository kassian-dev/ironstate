<p align="center">
  <img src="https://raw.githubusercontent.com/kassian-dev/ironstate/main/assets/wordmark.png" alt="ironstate" width="360">
</p>

<p align="center"><strong>Verified state machines for humans and AI agents.</strong></p>

[![crates.io](https://img.shields.io/crates/v/ironstate.svg)](https://crates.io/crates/ironstate)
[![docs.rs](https://img.shields.io/docsrs/ironstate)](https://docs.rs/ironstate)
[![CI](https://github.com/kassian-dev/ironstate/actions/workflows/ci.yml/badge.svg)](https://github.com/kassian-dev/ironstate/actions/workflows/ci.yml)
[![license](https://img.shields.io/crates/l/ironstate.svg)](#license)

A family of Rust crates for state machines you can trust: declare your states and transitions once, enforce them at runtime, and verify them in your tests — then build deterministic, replayable aggregates and an event journal on top.

## Why ironstate?

A **state machine** is just the set of states something can be in plus the legal
moves between them. An order goes `placed → paid → shipped`, and you must not
ship before payment. Most code writes this as scattered `if`s and `match`es, and
that's where the bugs hide: an illegal transition slips through, a case goes
unhandled, the "this can't happen" happens.

ironstate lets you declare the states and the rules once, in plain Rust, and
then does the part that usually gets skipped:

- **It enforces the rules at runtime.** An illegal move comes back as a clear,
  typed error instead of quietly going through. That's a guarantee, not a
  convention you hope everyone remembers.
- **It tests the rules for you.** From the same definition it generates thousands
  of scenarios to check your rules hold, and walks the state graph looking for
  dead ends and unreachable states. The definition is the test, so there's no
  separate model to keep in sync.

If your system changes through a *stream of events* (a bank ledger, a
multiplayer match, an approval workflow), the aggregate and journal crates add
three things those systems usually build by hand and get subtly wrong:

- **Deterministic replay.** The same events always rebuild the exact same state,
  down to the byte, so history is auditable and bugs are reproducible.
- **Per-viewer secrecy (redaction).** Show each player only what they're allowed
  to see — their own hand, not their opponent's — enforced by the type system.
- **An append-only event log** you can replay, fork, and resume: the durable
  record of what actually happened.

You write ordinary enums and a `match`; ironstate handles the enforcement, the
testing, and the bookkeeping. The family splits along one line: lifecycle
machines may react to transitions, while aggregates only record events. That
keeps the two from quietly contaminating each other.

| Crate | What it is |
|-------|------------|
| [`ironstate`](app/crates/ironstate) | Core lifecycle machines: enum states, one event → one hop, structural enforcement, listeners, a verification ladder |
| [`ironstate-derive`](app/crates/ironstate-derive) | The `StateMachine` / `Event` derive macros |
| [`ironstate-aggregate`](app/crates/ironstate-aggregate) | Deterministic aggregates: struct state, command → events via decide/evolve, journaled entropy, redaction, StableHash |
| [`ironstate-journal`](app/crates/ironstate-journal) | Event journal: append/snapshot/replay/fork, entropy positions, versioned upcasting, subscriptions, seeded simulation |

New here? The **[guide](docs/guide.md)** is a step-by-step walkthrough from a
first machine to a deterministic, event-sourced aggregate. API reference is on
docs.rs (one page per crate); build it locally with `make doc`.

---

## `ironstate` — core lifecycle machines

### Define a machine

Three pieces: a state enum (`#[derive(StateMachine)]`), an event enum
(`#[derive(Event)]`), and one pure transition function. The runtime enforces the
declared structure — terminal states reject everything, restricted states reject
events of the wrong kind — *before* your transition function runs.

```rust
use ironstate::prelude::*;

#[derive(StateMachine, Clone, Debug, PartialEq)]
#[state_machine(initial = Draft, terminal = [Archived])]
enum Article { Draft, Review, Published, Archived }

#[derive(Event, Clone, Debug, PartialEq)]
enum Edit { Submit, Approve, Reject, Archive }

impl TransitionRules for Article {
    type Event = Edit;
    fn transition(&self, event: &Edit) -> Option<Article> {
        use Article::*;
        use Edit::*;
        match (self, event) {
            (Draft, Submit) => Some(Review),
            (Review, Approve) => Some(Published),
            (Review, Reject) => Some(Draft),
            (Published, Archive) => Some(Archived),
            _ => None,
        }
    }
}

let mut m = Machine::<Article>::new();          // starts in Draft
assert_eq!(m.apply(Edit::Submit).unwrap(), Article::Review);

// Three ways to look before you leap, cheapest first:
assert!(m.could_apply(&Edit::Approve));         // bool
assert!(m.why_not(&Edit::Submit).is_some());    // the exact typed rejection, or None
assert_eq!(m.peek_transition(&Edit::Approve), Some(Article::Published)); // the target

// On rejection the event moves into the error, so you get it back without a clone:
let err = Machine::<Article>::restore(Article::Archived).apply(Edit::Submit).unwrap_err();
assert!(matches!(err, TransitionError::TerminalState { .. }));
assert_eq!(err.into_event(), Edit::Submit);
```

### Verification: `analyze!` and `test!`

The definition *is* the test. Drop these in a `#[cfg(test)]` module; each
expands to a `#[test]`.

**`analyze!`** walks the variant-level state graph and **proves structural
facts** — failing the build on design errors and reporting the rest. It needs no
runtime; it is pure graph analysis. Every line it prints is labeled `[proven]`
(holds by construction) or `[sampled]` (depends on the data a variant carries,
which `test!` exercises instead) — there are no unlabeled claims.

```rust,ignore
ironstate::analyze!(Article);
```

```text
→(Fe) ironstate analysis of `Article`
  · all 4 variants are reachable from Draft [proven]
  · no dead transitions [proven]
  · coverage: 4 of 16 (state, event) pairs produce transitions — variant-level; … [sampled]
```

It **fails** the test on: an unreachable state, a deadlock (a non-terminal state
with no way out), an inescapable cycle (a state that can never reach a terminal),
or a *dead transition* (one the transition function defines but structural
enforcement could never let fire). It caught a real bug while this repo was
being built — a state declared terminal that still had an outgoing transition.

**`test!`** generates random event sequences and, after every step, checks that
declared invariants hold and nothing panicked. On a violation, proptest shrinks
to the minimal failing sequence. Runs are reproducible with a `seed`.

```rust,ignore
ironstate::test!(Article);                            // defaults: 500 cases
ironstate::test!(Article, cases = 1000, max_steps = 50);
ironstate::test!(Article, seed = 0xDEC0DE);           // reproducible
```

Invariants are optional and declared via the `Invariants` trait; `test!` runs
either way (it always checks structural enforcement and absence of panics):

```rust,ignore
impl Invariants for Article {
    fn invariants() -> Vec<Invariant<Self, Self::Event>> {
        vec![Invariant::custom("archived is permanent")
            .assert(|before, _event, after| {
                before != &Article::Archived || after.is_none()
            })]
    }
}
```

### Event kinds and versioned restore

States can restrict *which kinds of event* they accept — for states driven by an
external system or gated behind an operator — and persisted machines can migrate
forward through a chain of versions on load.

```rust
use ironstate::prelude::*;
use serde::{Serialize, Deserialize};

#[derive(StateMachine, Serialize, Deserialize, Clone, Debug, PartialEq)]
#[state_machine(initial = Draft, terminal = [Retired], version = 3, history = [DocV1, DocV2])]
enum Doc { Draft, Live, Retired }

# #[derive(Event, Clone, Debug, PartialEq)] enum DocEvent { Publish, Retire }
# impl TransitionRules for Doc {
#     type Event = DocEvent;
#     fn transition(&self, e: &DocEvent) -> Option<Doc> {
#         use Doc::*; use DocEvent::*;
#         match (self, e) { (Draft, Publish) => Some(Live), (Live, Retire) => Some(Retired), _ => None }
#     }
# }
# #[derive(Serialize, Deserialize, Clone, Debug, PartialEq)] enum DocV1 { Draft, Live }
# #[derive(Serialize, Deserialize, Clone, Debug, PartialEq)] enum DocV2 { Draft, Live, Retired }
// `history` lists the retired types oldest-first; the derive requires a
// contiguous `MigrateFrom` chain at compile time.
impl MigrateFrom<DocV1> for DocV2 { fn migrate(o: DocV1) -> DocV2 { match o { DocV1::Draft => DocV2::Draft, DocV1::Live => DocV2::Live } } }
impl MigrateFrom<DocV2> for Doc  { fn migrate(o: DocV2) -> Doc  { match o { DocV2::Draft => Doc::Draft, DocV2::Live => Doc::Live, DocV2::Retired => Doc::Retired } } }

// A `{version, payload}` envelope written by any past version decodes and
// migrates to the current schema; a too-new version is a typed RestoreError.
let bytes = serde_json::to_vec(&serde_json::json!({ "version": 1, "payload": "Live" })).unwrap();
let m = Machine::<Doc>::restore_versioned(&bytes).unwrap();
assert_eq!(m.state(), &Doc::Live);
```

Restricting events by kind (`#[only_accepts(kind = "external")]` on a state,
`#[event_kind = "external"]` on an event) makes the runtime reject mismatched
events with a typed `EventKindRejected` before the transition function runs.

## `ironstate-aggregate` — deterministic aggregates

A struct changed by a `decide`/`evolve` pair: `decide` validates intent, draws
any entropy, and emits events; `evolve` applies one event, total and infallible.
Identical `(initial state, events)` replays bit-for-bit. Adds redaction
(`#[hidden]` fields with per-viewer views), a frozen canonical `StableHash`
digest, and the `test!`/`determinism_test!`/`leak_test!` macros.

See the [crate README](app/crates/ironstate-aggregate/README.md) for a runnable
`decide`/`evolve` example and the hidden-information walkthrough.

```rust,ignore
ironstate_aggregate::determinism_test!(MatchState);                  // two seeded runs must agree
ironstate_aggregate::leak_test!(MatchState, excluding = [PlayCard]); // no covert hidden → view flow
```

## `ironstate-journal` — the event journal

Persists events with the entropy position they consumed, so an aggregate can be
replayed, forked, and resumed bit-identically. A reference in-memory journal
passes a seven-property conformance suite (`journal_contract_test!`) every
storage adapter is judged against, and a seeded fault-schedule simulation
(`scenario_test!`) checks faults are invisible to outcomes. See the
[crate README](app/crates/ironstate-journal/README.md).

## Examples (end-to-end tests)

Each example under [`app/crates/examples/`](app/crates/examples) is a runnable
demo whose test module is an end-to-end test of a realistic use case:

- **`hidden-info`** — a hidden-information card match: redaction, journaled
  entropy, a system timeout, and a subscription. The redaction integration template.
- **`release-pipeline`** — a CI/CD release as a core lifecycle machine:
  operator- and external-gated states, declared invariants, `analyze!`/`test!`.
- **`ledger`** — an account as an aggregate over a journal: deposits/withdrawals,
  a non-negative-balance invariant, `execute`/`resume`, and `determinism_test!`.

---

## Determinism

The determinism contract is enforced by executable conditions, not prose: the
`StableHash` derive rejects floats, hash maps, and wall clocks in state at
compile time; the `EntropySource` API has no float or clock method; and
`determinism_test!` fails if two identically-seeded runs ever diverge.

---

## Development

The Cargo workspace lives under [`app/`](app/); the root `Makefile` drives cargo
there.

**Prerequisites**

- [`rustup`](https://rustup.rs). The workspace tracks the latest stable toolchain
  via `app/rust-toolchain.toml`; `rustup` installs it on first build.
- `rustup target add wasm32-unknown-unknown` for `make wasm`, which builds the
  determinism-sensitive crates for wasm32 to prove they have no host coupling.
- Optional: `cargo install cargo-deny` (the supply-chain gate).

**Common tasks** (`make help` lists them all)

```sh
make build    # build the workspace
make test     # cargo test --workspace --all-features
make check    # the done-gate: fmt-check + clippy (-D warnings) + test
make doc      # build the rustdoc with warnings denied
make wasm     # cross-target build for wasm32
make deny     # licenses / advisories / duplicate majors
make msrv     # build on the minimum supported Rust (1.96)
make fuzz     # fuzz the restore-decode path (needs nightly + cargo-fuzz)
make mutants  # mutation-test the code (cargo-mutants)
```

`make check` is the single done-gate — the same for a human at a keyboard and an
agent in a loop.

**Workflow.** Documents are law: design intent is written down
([`docs/design.md`](docs/design.md)), code implements it, tests cite it. The
order is doc change → code → tests → gates. See [`AGENTS.md`](AGENTS.md),
[`docs/testing.md`](docs/testing.md), and [`docs/decisions/`](docs/decisions).

## Releasing & supply-chain security

Publishing is CI-driven. On a push to `main`,
[release-plz](https://release-plz.dev) (in
[`.github/workflows/release.yml`](.github/workflows/release.yml)) opens a release
PR that bumps the versions of the crates whose source changed and updates the
[CHANGELOG](CHANGELOG.md); merging it publishes **only those crates**, in
dependency order, and tags each. The `examples/*` crates are `publish = false`,
and release-plz maintains the changelog (the hand-written `0.1.0` entry seeds it).

Security practices, all in that workflow:

- **Trusted Publishing (OIDC)** — crates.io issues a short-lived token at
  runtime, so there is **no long-lived `CARGO_REGISTRY_TOKEN` secret**. (One-time
  setup: register this repo + workflow as a trusted publisher per crate, after
  each crate's first manual publish.)
- **Build-provenance attestations** — published artifacts get SLSA provenance
  signed via Sigstore and logged in the public Rekor transparency log.
- **Signed release tags** and **SHA-pinned actions**.
- **`cargo deny`** (licenses, advisories, yanked, duplicate majors) gates every
  PR, and `Cargo.lock` is committed.

crates.io does not verify per-crate signatures today, so the transparency story
is Trusted Publishing + provenance + signed tags rather than a crate signature.

## License

Apache-2.0.
