# 0002 — Deferred and out of scope

Recorded so these are not re-derived later. Status: deferred unless noted.

## D-1 — Dropped: the dylint determinism lints

The original design included `ironstate-lints`, a dylint `cdylib` with three
lints (no floats in aggregate state, no unordered iteration in `decide`/`evolve`,
no wall-clock reads). **We chose not to build it.**

Rationale: the determinism contract those lints enforce is *already* executable
through tested mechanisms — the `StableHash` derive rejects floats / hash maps /
wall clocks in state at compile time (covered by `trybuild` fixtures), the
`EntropySource` API has no float or clock method, and `determinism_test!` fails
on any non-determinism in `decide`/`evolve` (covered by a planted-defect
fixture). The lints would only add lint-time feedback for a violation a consumer
forgot to test — at the cost of a `cdylib` linking rustc internals on a pinned
nightly, with ongoing churn as those internals drift, and an extra
`cargo dylint` CI step. Not worth it for the marginal coverage; consumers run on
**stable**. Revisit only if a consumer asks for lint-time feedback.

## D-2 — Deferred: the aggregate stateright bridge (`BranchingEntropy`)

Exhaustive model checking for aggregates. Activates when a consumer names a
liveness property they need *proven* (a match clock forcing "always eventually
terminal"; "the active actor always has a legal move"). The recorded design:
entropy becomes branching nondeterminism to explore rather than a stream to
sample; the consumer supplies shrunk bounds and a command enumerator (`why_not`
makes legal-command enumeration cheap); `Digest128` is the state-dedup
fingerprint; results carry a new `[proven@bounds]` label. A first activation
review found no surviving property in the candidate ruleset (termination and
legality were structural), so it stays deferred.

## D-3 — Deferred: cross-journal invariants via a multi-journal reference run

Properties that span more than one aggregate's journal (e.g. conservation across
matches). Evidence-gated; likely an extension of the subscription/scenario
reference-run machinery rather than the stateright bridge.

## D-4 — Deferred: the proptest `subscription_test!`

The in-process `Subscription` mechanism and its idempotency property are built
and tested (duplicates and out-of-order redeliveries are dropped, converging to
exactly-once). A proptest macro over a *generated* source stream with
fault-injected redelivery is the natural enhancement, sharing the
`scenario_test!` fault machinery.

## D-5 — Out of scope (downstream or unneeded)

- **Storage adapters** (Postgres, etc.) — downstream, against the `Journal`
  trait and the contract suite, until a second consumer wants one.
- **Message transport / durable delivery (outboxes)** — application code.
- **Commit–reveal / seed-commitment protocols** — built *on* `AuditDigest` by
  applications; no seed-commitment API in the family.
- **Event-level redaction** — the view-distribution model (clients consume
  views, not raw events) removes the need; revisit only with evidence.
- **A generic snapshot-cadence policy engine** — snapshot when you like; cadence
  is application policy, not a trait method.
- **Kani for aggregates** — state-space explosion over struct state needs its
  own design.

## Normative policies we keep

- **Frozen-encoding rule.** The canonical `StableHash` encoding is frozen at
  first release; any change is a new additive encoding version, never a mutation
  of v1, because published `AuditDigest`s are immortal. Golden vectors enforce it.
- **Bump-on-shape-change versioning.** *Any* change to a wire shape (additive
  variants included) bumps the version, with the trivial identity `MigrateFrom`
  as the cost — additive-without-bump silently depends on deploy gating, which is
  invisible to the type system and untestable.
- **Sans-I/O.** Time enters as events or `LogicalTime` data; I/O lives in
  adapters. The pure core is what makes the whole verification ladder possible.
