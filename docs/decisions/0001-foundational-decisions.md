# 0001 — Foundational decisions

Status: accepted. These record where the implementation deliberately fills a gap
the specs left, or departs from them, so the choices are not re-litigated later.

## F-1 — Rust lives under `app/`, Makefile at the root

The build guide puts the Cargo workspace at the repo root. Per maintainer
preference the workspace lives under `app/` and the `Makefile` stays at the root
as a thin control surface. Specs, `AGENTS.md`, and `docs/` are at the root.

## F-2 — Toolchain tracks latest stable, not a pinned version

Per maintainer preference `app/rust-toolchain.toml` uses `channel = "stable"`, so
day-to-day development tracks the latest stable compiler. Edition is 2024; the
workspace `rust-version` (1.96) is an MSRV floor, not a pin — the `msrv` CI job
builds the workspace on exactly 1.96.0 so the declared floor stays honest.

## F-3 — Event enums require `#[derive(Event)]`

v0.4 sketches events as plain enums with `#[event_kind]` / `#[likelihood]`
attributes. Those attributes are not legal on a plain enum — a derive must
register and read them. So events derive `Event`, which generates the
`EventKind` impl (kinds, variant names, variant enumeration, likelihood
weights). `TransitionRules::Event` is bound `EventKind + Clone + Debug` so the
requirement is uniform and the bound is implied wherever a machine is used.

## F-4 — Variant enumeration for analysis uses `Default`

`analyze!` and `test!` walk every state and event variant. The derives build one
representative per variant; data-carrying variants fill their fields with
`Default::default()`, since analysis and generation are variant-level. A state or
event type that carries data must therefore implement `Default` (or hand-write
the `StateMachine`/`Event` impl). Phase machines — the aggregate tier's use of
core — are fieldless, so this is free there.

## F-5 — Versioned restore (and its serde dependency) lives behind a default-on `restore` feature

The derive-generated `restore_versioned` decodes a `{version, payload}` JSON
envelope, which needs `serde`/`serde_json`. Rather than make those unconditional
dependencies of `ironstate`, they sit behind a `restore` feature that is on by
default: the common case is unchanged, but a consumer using core as a plain
in-memory state machine can set `default-features = false` and carry no JSON
parser. Because generated `Versioned` code calls into core's runtime, the higher
tiers forward the switch — `ironstate-aggregate`'s own `restore` feature enables
`ironstate/restore`. The `Versioned` impl is still generated only when
`version`/`history` is declared, so unversioned types need neither the feature
nor a `Deserialize` bound.

## F-6 — `anyhow` for glue, typed errors at library boundaries

`anyhow` is used in application/binary/test glue (the example's `main`, harness
code). Library public errors stay typed (`thiserror`-style) so consumers map
what/why/fix straight off the variants — the family's typed-error contract.

## F-7 — Deferred core rungs (not implemented)

`model_test!`, the stateright bridge, Kani harnesses, Mermaid output, formal
mathematical output, and async listeners are part of the original design but are
not implemented. None are load-bearing for the aggregate/journal milestones. The
no-op feature flags that once stood in for them were removed so the published
crate does not advertise capabilities it lacks; when one is built it ships with
its feature flag and tests in the same change.

## F-8 — The phase machine is reused as a definition, not held as a live `Machine`

The spec describes the aggregate runtime as "holding a `Machine<A::Phase>`
internally." In practice the aggregate's `evolve` owns the phase (it is a field
of the state), so a parallel live `Machine<A::Phase>` would be redundant state
to keep in sync. Instead the runtime reuses the phase type's `StateMachine`
trait directly — `phase.is_terminal()` and `phase.restriction()` are the same
single-branch structural checks core's `apply` performs. Phase-hop legality
(every `evolve`-produced phase change is a legal transition of the phase
machine) is verified by the aggregate `test!` macro, not by driving a held
instance.

## F-9 — `AggregateRules::Ctx` is owning, not borrowing

`type Ctx` is a plain associated type, so a borrowing `DeterministicCtx<'a,
Actor>` cannot be used as `Ctx` — the lifetime would be unconstrained by the
impl. The contract context is therefore the owning `OwnedDeterministicCtx<Actor>`
(it owns `Box<dyn EntropySource>`). The borrowing `DeterministicCtx<'a, Actor>`
remains a call-site convenience, and its `probing()` yields the owning form for
`why_not`. The persistent loop (`execute`, journal milestone) threads the live
entropy through the owning context and reads/rewinds positions via `CtxEntropy`.

## F-10 — `resume` returns `ResumeError`, not bare `RestoreError`

The spec types `resume` as returning `RestoreError`. But resuming reads the
journal (which fails with `JournalError`), replays (which fails with
`RestoreError` on a version mismatch), and may find no base snapshot. Core's
`RestoreError` is `#[non_exhaustive]` and cannot be extended from the journal
crate, so `resume` returns a `ResumeError { Journal | Restore | NoBase }` that
names all three causes honestly. `replay` and `replay_hash` still return the
core `RestoreError` directly, as the spec specifies.

## F-11 — Fuzzing blocks; mutation testing is advisory; both run per PR and on main

Untrusted bytes enter ironstate in exactly one place: `restore_versioned` decodes
a stored `{version, payload}` envelope. A `cargo-fuzz` target exercises that path
on every pull request and blocks on a crash, since a crash is a real,
reproducible bug. Because the run is non-deterministic, a found crash is folded
into the corpus (and uploaded) so libFuzzer replays it every run and re-fails
until fixed — a known crash can't silently flip green on a later push; the
lasting gate is a regression test built from the reproducer. Fuzzing needs a
nightly toolchain, isolated to that one CI job — the crates themselves stay on stable. Mutation testing
(`cargo-mutants --in-diff`) runs on the changed lines and is advisory: it posts
surviving mutants to a sticky PR comment but never fails the build, because
equivalent mutants would otherwise block unrelated work. The comment updates in
place — it never deletes (delete/recreate would flicker under the run-to-run
non-determinism), and a generous per-mutant timeout keeps results deterministic
so it does not falsely flip to "clean". Both live in `quality.yml`, kept separate
from `ci.yml` so the elevated `pull-requests` permission stays scoped to it.
Blocking on mutants can be revisited once the equivalent-mutant excludes settle.
