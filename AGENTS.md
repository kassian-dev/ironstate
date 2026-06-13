# Contributing to ironstate (humans and agents)

**What this is.** Ironstate is a Rust workspace of verified state machines: a
core lifecycle-machine crate plus an aggregate and journal family built on it.
**Documents are law** — design intent is written down; code implements it; tests
cite it. When code and a doc disagree, the code is wrong until the doc is amended.

## Reading list

- `docs/guide.md` — a step-by-step tutorial for newcomers (build a machine, an
  aggregate, a journal). The gentlest entry point.
- `docs/design.md` — the family's design, distilled: the two paradigms, the
  verification ladder, the determinism contract, the vocabulary.
- `docs/testing.md` — the test taxonomy: what each layer proves and where it lives.
- The README's "Releasing & supply-chain security" section — how publishing works.
- `docs/decisions/` — why the code is shaped the way it is, including what was
  deliberately deferred or dropped. A fresh context should be able to reconstruct
  every "why" from these files, not from chat.

## Engineering standard

Documents are law; one source of truth per domain; machine-checkable beats prose
(every invariant has a test); honest state (debt and deferrals are recorded in
`docs/decisions/` with their resolution, never silent); one done-gate for
everyone (`make check`). Generated artifacts are never hand-edited. Decision
records are files in the repo, not chat history.

## Non-negotiable invariants (each with the test that enforces it)

- **Never panic on input or external failure.** Every fallible operation returns
  a typed error; `evolve` (aggregate tier) is the one infallible-by-contract
  function. Enforced by the error-path tests and `cargo-fuzz` on decode paths.
- **Teaching errors.** Every `Display`, every derive diagnostic: what happened,
  why, what to do. Enforced by error-message snapshot tests and `trybuild`.
- **Determinism is conditional and the conditions are executable.** Same
  `(seed, journal)` ⇒ identical `Digest128` across targets. Enforced by
  the `StableHash` derive's compile errors (floats / hash maps / wall clocks in
  state are rejected), the float- and clock-free `EntropySource` API,
  `determinism_test!` (catches any nondeterminism in `decide`/`evolve`), and
  `journal_contract_test!`.
- **Golden vectors are write-once.** Entropy streams and digests are generated
  with the code under test and never regenerated to make a red test green.
- **Honest claims.** Every analysis/test-macro claim is labeled `[proven]` or
  `[sampled]`. No unlabeled claims.

## Done-gate

```
make check        # fmt-check + clippy (-D warnings) + test --all-features
```

Agents and humans use the same gate. There is no looser "agent mode." On every
pull request, `quality.yml` additionally fuzzes the restore-decode path
(`make fuzz`, blocks on a crash) and mutation-tests the changed code
(`make mutants --in-diff`, advisory — a sticky PR comment, never blocks).

## Workflow order

doc change → code → tests → gates. If you are adding behavior the specs do not
describe: stop and amend the spec (or file a decision record) first.

## Layout

- `app/` — the Cargo workspace (maintainer's choice to keep Rust under `app/`).
  - `app/crates/ironstate` — core lifecycle machines.
  - `app/crates/ironstate-derive` — `StateMachine` / `Event` derives.
  - `app/crates/ironstate-aggregate` (+ `-derive`) — decide/evolve, entropy,
    redaction, StableHash.
  - `app/crates/ironstate-journal` — append/replay/fork/subscribe.
  - `app/crates/examples/` — runnable examples that double as end-to-end tests:
    `hidden-info` (the redaction integration template), `release-pipeline` (a
    core lifecycle machine), and `ledger` (an aggregate over a journal).
- `Makefile` — universal verb interface at the repo root; drives cargo in `app/`.
- `docs/` — specs, standards, testing taxonomy, decision records.

## Counter-intuitive things (don't "helpfully" undo)

- **Toolchain tracks latest stable** (`app/rust-toolchain.toml`), not a pinned
  version — by maintainer preference.
- **Code comments stand on their own** — they never cite spec/doc section
  numbers. Explain the reason in the comment itself.
- **Event enums require `#[derive(Event)]`.** The `#[event_kind]` /
  `#[likelihood]` attributes need a derive to be legal and read; this is a
  deliberate addition over the spec's "plain enum" sketch.
- **Versioned restore sits behind a default-on `restore` feature.** It pulls
  `serde`/`serde_json` to decode the envelope; a lean consumer can turn it off
  with `default-features = false`. Higher tiers forward it (`ironstate-aggregate`'s
  `restore` enables `ironstate/restore`).
- **`anyhow` is for application/binary/test glue only.** Library public errors
  stay typed (`thiserror`-style) so consumers map what/why/fix off variants.
- See `docs/decisions/0001-foundational-decisions.md` for the full list with
  rationale.
