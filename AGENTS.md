# Contributing to ironstate (humans and agents)

**What this is.** Ironstate is a Rust workspace of verified state machines: a
core lifecycle-machine crate plus an aggregate and journal family built on it.
**Documents are law** — design intent is written down; code implements it; tests
cite it. When code and a doc disagree, the code is wrong until the doc is amended.

## Reading list

- `docs/README.md` — the documentation index and the reading order for everything below.
- `docs/guide.md` — a step-by-step tutorial for newcomers (build a machine, an
  aggregate, a journal). The gentlest entry point.
- `docs/design.md` — the family's design, distilled: the two paradigms, the
  verification ladder, the determinism contract, the vocabulary.
- `docs/testing.md` — the test taxonomy: what each layer proves and where it lives.
- The README's "Releasing & supply-chain security" section — how publishing works.
- "Counter-intuitive things" and "What deliberately doesn't exist", below — why
  the code is shaped the way it is, and what was deferred or dropped on purpose.
  A fresh context should be able to reconstruct every "why" from the repo, not
  from chat.

## Engineering standard

Documents are law; one source of truth per domain; machine-checkable beats prose
(every invariant has a test); honest state (debt and deferrals are recorded
under "What deliberately doesn't exist" with their trigger, never silent); one
done-gate for everyone (`make check`). Generated artifacts are never
hand-edited. Rationale lives in files in the repo, not chat history.

## Non-negotiable invariants (each with the test that enforces it)

- **Never panic on input or external failure.** Every fallible operation returns
  a typed error; `evolve` (aggregate tier) is the one infallible-by-contract
  function. Enforced by the error-path tests and `cargo-fuzz` on decode paths.
- **Teaching errors.** Every `Display`, every derive diagnostic: what happened,
  why, what to do. Enforced by error-message snapshot tests and `trybuild`.
- **Docs teach, with runnable examples.** Every public item carries rustdoc —
  what and why — plus a runnable example (a doctest, so it cannot rot) wherever
  it helps the adopter on docs.rs; follow the rustdoc conventions
  (`# Examples` / `# Panics` / `# Errors`, intra-doc links over bare names).
  Adopter DX is first-class. The gate enforces the machine-checkable parts:
  `#![warn(missing_docs)]` in every published crate (under `-D warnings`) so
  nothing public ships undocumented, the `cargo test` doctests so every example that exists
  compiles and runs, and the `-D warnings` rustdoc build so links resolve.
  *Whether* an item that would help the adopter carries an example is a review
  call, not a lint.
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
pull request, `fuzz.yml` additionally fuzzes the restore-decode path
(`make fuzz`, blocks on a crash). Mutation-testing the changed code
(`make mutants --in-diff`, advisory) is a separate manual workflow, `mutants.yml`.

**Why they are gated differently.** Untrusted bytes enter ironstate in exactly
one place — `restore_versioned` decoding a stored `{version, payload}` envelope
— so fuzzing that path **blocks**: a crash there is a real, reproducible bug.
Because the run is non-deterministic, a found crash is folded into the corpus
(and uploaded) so libFuzzer replays it every run and re-fails until fixed; the
lasting gate is a regression test built from the reproducer. Fuzzing needs a
nightly toolchain, isolated to that one CI job — the crates themselves stay on
stable. Mutation testing is **advisory**: it reports survivors to the run summary
but never fails the build, because equivalent mutants would otherwise block
unrelated work. A generous per-mutant timeout keeps results reproducible (a
survivor drops off only when code or tests change, not from build timing). It is
manual because most PRs have little to mutate; trigger it when changing core
logic. Blocking on mutants can be revisited once the equivalent-mutant excludes
settle.

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
  - `app/crates/ironstate/fuzz/` — the cargo-fuzz target for the restore-decode
    path; its own detached workspace, so it stays out of `--workspace` builds.
  - `app/crates/examples/` — runnable examples that double as end-to-end tests,
    indexed in `app/crates/examples/README.md`: `hidden-info` (the redaction
    integration template), `release-pipeline` (a core lifecycle machine),
    `ledger` (an aggregate over a journal), `catalog-ctx` (adopting the owned
    `Ctx`), and `async-store` (an async adapter held under contract by a sync
    twin). Each example's `main.rs` pulls its prose from its own `README.md` via
    `#![doc = include_str!("../README.md")]`, so the page and the crate docs
    never drift.
- `Makefile` — universal verb interface at the repo root; drives cargo in `app/`.
- `docs/` — specs, standards, testing taxonomy, decision records.
- `assets/` — brand assets (logo, wordmark, social card, favicon), generated by
  `assets/brand.py` and rendered with resvg; the SVGs/PNGs are generated, never
  hand-edited (see `assets/README.md`).
- `.github/workflows/` — `ci.yml` (the done-gate in CI), `fuzz.yml` (per-PR
  fuzzing), `mutants.yml` (manual mutation testing), `release-pr.yml` (dispatched manually to open the
  release-plz version-bump/changelog PR) and `release.yml` (publishes to
  crates.io on push to `main`, i.e. when that PR merges; a no-op otherwise).

## Adding an example

Examples are the family's anti-drift gate, and their listing lives in several
places — update all of them in the same change, or the next contributor inherits
a stale map (this has been missed before):

1. `app/Cargo.toml` — add the crate to `members` (the list is explicit, not a glob).
2. `app/crates/examples/<name>/README.md` — the example's own page; `main.rs`
   carries it as crate docs with `#![doc = include_str!("../README.md")]`, so
   write the prose once, there. Keep it GitHub-readable (examples are
   `publish = false` and never reach docs.rs, so prefer plain code spans over
   rustdoc intra-doc links).
3. `app/crates/examples/README.md` — add a row to the index table (tier, what it
   shows, test layers, when to start here).
4. `README.md` — add it to the "Examples" section.
5. `docs/testing.md` — add it to "Examples as end-to-end tests".
6. `docs/guide.md` — only if it mirrors the guide's tutorial arc; adoption-only
   examples (`catalog-ctx`, `async-store`) stay out of the beginner path.

## What deliberately doesn't exist

Recorded so it isn't re-derived, re-proposed, or "helpfully" added. Each entry
says why it isn't there and what would change that.

**Dropped.**

- **The dylint determinism lints** (`ironstate-lints`: no floats in aggregate
  state, no unordered iteration in `decide`/`evolve`, no wall-clock reads). The
  contract they would enforce is *already* executable — the `StableHash` derive
  rejects floats / hash maps / wall clocks at compile time (`trybuild` fixtures),
  the `EntropySource` API has no float or clock method, and `determinism_test!`
  fails on any non-determinism in `decide`/`evolve` (planted-defect fixture).
  The lints would add only lint-time feedback for a violation a consumer forgot
  to test, at the cost of a `cdylib` linking rustc internals on a pinned nightly,
  churn as those internals drift, and an extra CI step. Consumers run on
  **stable**. *Revisit only if a consumer asks for lint-time feedback.*

**Deferred — designed, not built.**

- **Core rungs**: `model_test!`, the stateright bridge, Kani harnesses, Mermaid
  output, formal mathematical output, async listeners. None are load-bearing for
  the aggregate/journal milestones. The no-op feature flags that once stood in
  for them were removed so the published crates do not advertise capabilities
  they lack; when one is built it ships with its feature flag and tests in the
  same change.
- **The aggregate stateright bridge (`BranchingEntropy`)** — exhaustive model
  checking for aggregates. Recorded design: entropy becomes branching
  nondeterminism to explore rather than a stream to sample; the consumer supplies
  shrunk bounds and a command enumerator (`why_not` makes legal-command
  enumeration cheap); `Digest128` is the state-dedup fingerprint; results carry a
  new `[proven@bounds]` label. *Activates when a consumer names a liveness
  property they need proven* (a match clock forcing "always eventually terminal";
  "the active actor always has a legal move"). A first activation review found no
  surviving property in the candidate ruleset — termination and legality were
  structural.
- **Cross-journal invariants** spanning more than one aggregate's journal (e.g.
  conservation across matches). Evidence-gated; likely an extension of the
  subscription/scenario reference-run machinery rather than the stateright
  bridge.
- **The proptest `subscription_test!`** over a *generated* source stream with
  fault-injected redelivery. The in-process `Subscription` and its idempotency
  property are built and tested (duplicates and out-of-order redeliveries are
  dropped, converging to exactly-once); the macro would share the
  `scenario_test!` fault machinery.
- **Kani for aggregates** — state-space explosion over struct state needs its own
  design.

**Out of scope — downstream, or unneeded.**

- **Storage adapters** (Postgres, SQLite, …) — still downstream, written against
  the `Journal` trait and held to `journal_contract_test!`. The "until a second
  consumer wants one" gate **has now fired** (a relational adopter arrived
  wanting one), which is what motivated the multi-stream and transactional
  reshape of `Journal` — but the adapters themselves stay out of this repo.
- **Message transport / durable delivery (outboxes)** — application code.
  ironstate does not *implement* an outbox. It does provide the **seam** one
  needs: `Journal::Tx` plus `execute_in`, so an adopter can enclose the append
  in their own transaction alongside their read-model and outbound writes. The
  distinction is the sans-I/O rule — an `append` that owned its commit would be
  ironstate making an I/O policy decision on the adapter's behalf.
- **Commit–reveal / seed-commitment protocols** — built *on* `AuditDigest` by
  applications; no seed-commitment API in the family.
- **Event-level redaction** — the view-distribution model (clients consume views,
  not raw events) removes the need. *Revisit only with evidence.*
- **A generic snapshot-cadence policy engine** — snapshot when you like; cadence
  is application policy, not a trait method.

## Counter-intuitive things (don't "helpfully" undo)

- **Toolchain tracks latest stable** (`app/rust-toolchain.toml`), not a pinned
  version — by maintainer preference. That is independent of the **MSRV floor**
  (`workspace.package.rust-version`), which is a *floor, not a pin*: the `msrv`
  CI job builds the workspace on exactly that version so the declared floor stays
  honest. The floor is set by language features actually used — currently 1.88,
  for let-chains — not by the edition (edition 2024 itself needs only 1.85) and
  not by dependencies (they sit at or below 1.85). Raise it only when a feature
  earns it; `resolver = "3"` is MSRV-aware and picks compatible dependency
  versions on its own.
- **Code comments stand on their own** — they never cite spec/doc section
  numbers. Explain the reason in the comment itself.
- **Crate-README links are absolute, repo-doc links are relative — on purpose.**
  A published crate README renders on crates.io and docs.rs, where relative paths
  break, so links in `app/crates/*/README.md` use absolute `github.com` /
  `crates.io` URLs. The root `README.md` and everything under `docs/` are
  GitHub-only and use relative paths. Don't unify the two styles.
- **Event enums require `#[derive(Event)]`.** The `#[event_kind]` /
  `#[likelihood]` attributes need a derive to be legal and read; this is a
  deliberate addition over the spec's "plain enum" sketch.
- **Versioned restore sits behind a default-on `restore` feature.** It pulls
  `serde`/`serde_json` to decode the envelope; a lean consumer can turn it off
  with `default-features = false`. Higher tiers forward it (`ironstate-aggregate`'s
  `restore` enables `ironstate/restore`).
- **`anyhow` is for application/binary/test glue only.** Library public errors
  stay typed (`thiserror`-style) so consumers map what/why/fix off variants.
- **Data-carrying fields must implement `Default`.** `analyze!` and `test!` walk every variant, and the derives build one representative per variant with `Default::default()` in its fields — analysis is variant-level, so the values are never the point. Fieldless enums (the aggregate tier's phase machines) need nothing. Documented for adopters in `docs/guide.md` and the derive rustdoc.
- **`resume` returns `ResumeError`, not bare `RestoreError`.** Resuming reads the
  journal (`JournalError`), replays (`RestoreError` on a version mismatch), and
  may find no base snapshot. Core's `RestoreError` is `#[non_exhaustive]` and
  cannot be extended from the journal crate, so `resume` names all three causes
  honestly. `replay` and `replay_hash` still return `RestoreError` directly.
- **Cross-target determinism is checked x86_64 vs aarch64; wasm32 is
  build-only.** The `test` matrix runs the seeded `determinism_test!` digests on
  both Linux architectures and the `determinism` job fails if the manifests
  differ — aarch64 because it is a first-class hosted runner with a different
  codegen backend, so the suite just runs there. wasm32 is the more obvious
  target and the encoding is designed to be wasm-identical, but running the
  seeded suite under wasm needs a wasm test runner (wasmtime + `wasm32-wasip1`,
  with proptest/getrandom wired up), so `make wasm` stays a *build-only* check:
  the determinism-sensitive crates must link with no host dependence, and an
  undefined wasm symbol is a hard error. The per-target
  `target/ironstate-determinism/*.digest` files are ephemeral CI artifacts
  (regenerated every run, gitignored) — distinct from the write-once golden
  vectors pinned in source.
