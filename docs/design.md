# Ironstate design

The enduring design of the family, distilled. This is the in-repo design
reference; the *why* behind specific implementation choices lives in
[`decisions/`](decisions), and the contributor workflow in
[`../AGENTS.md`](../AGENTS.md).

## What it is

Verified state machines as Rust enums, and a family of crates built on that
core. You define states, transitions, and invariants in Rust; the same code
runs in production and generates its own verification. There is no separate
model to keep in sync.

## The two paradigms (a crate boundary, not a flag)

- **Lifecycle machines** (`ironstate`): an enum of states, one event → one hop,
  structural enforcement, and listeners that fire *at* a transition. They **may
  listen** — observe and trigger side effects.
- **Aggregates** (`ironstate-aggregate`): struct state changed by a
  `decide`/`evolve` pair. They are pure; their only output is journaled events.
  They **may only emit** — there is no listener API, by construction.

Capabilities that would compromise one paradigm if reachable from the other are
separated by crates. Listeners exist only in core; entropy exists only in the
aggregate tier.

Dependency direction: `aggregate → core`, `journal → aggregate`. Each crate
publishes and versions independently.

## The aggregate's two laws

- `decide(&self, &Command, &mut Ctx) -> Result<Vec<Event>, Error>` is the only
  place rules live and the only function permitted to draw entropy. It validates
  intent and emits the facts that follow; it does not mutate state.
- `evolve(&mut self, &Event)` is total, infallible, and pure. It applies one
  fact, draws no entropy, and reads no clock.

Because replay is a sequence of `evolve`s and `evolve` draws nothing, **replay
consumes no entropy** — so the entropy position cannot be recomputed from events
and must be recorded with every append. That single fact shapes the journal.

## The verification ladder

One definition, several rungs of assurance, each labeled `[proven]` (holds by
construction) or `[sampled]` (observed over generated input):

- **Structural** — terminal states and event-kind restrictions are enforced by
  the runtime before the transition function runs (`[proven]`).
- **`analyze!`** — graph analysis: reachability, deadlocks, inescapable cycles,
  dead transitions (`[proven]`); coverage (`[sampled]`).
- **`test!`** — randomized property testing against declared invariants.
- **`determinism_test!`** — two identically-seeded runs must agree digest-for-digest.
- **`leak_test!`** — no covert flow from one principal's hidden state to another's view.
- **`journal_contract_test!`** — a journal adapter satisfies the seven contract properties.
- **`scenario_test!`** — seeded whole-tier fault simulation; faults invisible to outcomes.

## The determinism contract (conditions are executable)

Given state/events that hash stably, entropy drawn only through the journal-owned
source, and an adapter that passes the contract suite, identical `(seed,
journal)` produces an identical `Digest128` on `x86_64` and `wasm32`. The
conditions are not prose — they are enforced:

- The `StableHash` derive **rejects floats, hash maps, and wall clocks in state**
  at compile time, with a teaching error naming the fix.
- The `EntropySource` API has **no float and no clock method** — it cannot
  express a non-deterministic input.
- `determinism_test!` **fails** if `decide`/`evolve` is ever non-deterministic.
- The canonical encoding is **frozen**: integers little-endian in fixed width,
  `usize`/`isize` widened to 8 bytes so 32- and 64-bit targets agree, length
  prefixes and declaration-order discriminants so distinct values never collide.
  Golden vectors pin it forever.

Two digests, one encoding: `Digest128` (non-adversarial, detects accidental
divergence) and `AuditDigest` (BLAKE3, collision-resistant, the only digest ever
published — for commit–reveal and audit).

## Redaction

What one principal sees of another's state. `Conceal` declares the public
residue of an owned value; `OwnerRedact` produces a per-viewer view where the
viewer sees their own value in full and everyone else sees only the residue. The
generated view types *cannot represent* another principal's hidden value — the
exclusion is type-level. Raw events are trusted-side artifacts; live clients
consume per-principal views, and full-journal replay is a post-reveal activity.

## Versioning

Whole-enum versioning with a `version`/`history` grammar shared by core state
machines and journal events: the derive requires a contiguous `MigrateFrom`
chain at compile time, and a stored `{version, payload}` envelope is upcast to
the current schema on load. House policy: *any* change to the wire shape bumps
the version, with the trivial identity migration as the cost.

## Vocabulary

- **actor** — who issued a command (a consumer type, in `Ctx`).
- **principal** — who is looking at state (a consumer type, in `#[redact]`).
- **residue** — the `Conceal` output; declared public.
- **position** (`DrawPos`) — count of journaled entropy draws.
- **head** — the latest `Seq`.
- **probe** — an uncounted entropy fork for speculation (`why_not`).
- **`[proven]` / `[sampled]`** — holds by construction vs. observed over a corpus.
