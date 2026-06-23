# →(Fe) ironstate-aggregate

Deterministic aggregates for the [ironstate](https://docs.rs/ironstate) family:
struct state changed by a `decide`/`evolve` pair, with journaled entropy,
redaction, and stable hashing.

An **aggregate** is a consistency boundary. Two functions, two laws:

- `decide(&self, &Command, &mut Ctx) -> Result<Vec<Event>, Error>` — the only
  place rules live, and the only place entropy is drawn. It validates intent and
  emits the facts that follow. It does not mutate state.
- `evolve(&mut self, &Event)` — total, infallible, pure. It applies one fact.

Because `evolve` is pure and `decide` is the sole consumer of entropy, replaying
a `(initial state, events)` pair reproduces the state bit-for-bit.

Unlike a core lifecycle machine, an aggregate has **no listeners** — its only
output is the events it emits.

## Simple example

```rust
use ironstate::prelude::*;
use ironstate_aggregate::*;

#[derive(StateMachine, Clone, Debug, PartialEq)]
#[state_machine(initial = Open, terminal = [Closed])]
enum Phase { Open, Closed }

#[derive(Event, Clone, Debug, PartialEq)]
enum Step { Close }
impl TransitionRules for Phase {
    type Event = Step;
    fn transition(&self, _: &Step) -> Option<Phase> {
        matches!(self, Phase::Open).then_some(Phase::Closed)
    }
}

#[derive(Event, Clone, Debug, PartialEq)]
enum Command { Roll, Close }

#[derive(Clone, Debug, PartialEq)]
enum DieEvent { Rolled(u8), Closed }

#[derive(Debug, thiserror::Error)]
#[error("the table is closed")]
struct Closed;

#[derive(Clone, Debug, PartialEq)]
struct Table { phase: Phase, total: u32 }

impl AggregateRules for Table {
    type Phase = Phase;
    type Command = Command;
    type Event = DieEvent;
    type Error = Closed;
    type Ctx = OwnedDeterministicCtx<u32>;

    fn phase(&self) -> Phase { self.phase.clone() }

    fn decide(&self, cmd: &Command, ctx: &mut Self::Ctx) -> Result<Vec<DieEvent>, Closed> {
        if self.phase != Phase::Open { return Err(Closed); }
        Ok(match cmd {
            // The one place entropy is drawn — an unbiased 1..=6 die.
            Command::Roll => vec![DieEvent::Rolled(ctx.entropy.draw_range(1..7) as u8)],
            Command::Close => vec![DieEvent::Closed],
        })
    }

    fn evolve(&mut self, event: &DieEvent) {
        match event {
            DieEvent::Rolled(n) => self.total += u32::from(*n),
            DieEvent::Closed => self.phase = Phase::Closed,
        }
    }
}

let mut table = Aggregate::new(Table { phase: Phase::Open, total: 0 }).unwrap();
let mut ctx = OwnedDeterministicCtx {
    entropy: Box::new(SeededEntropy::from_seed(&Seed([7; 32]))),
    actor: 1,
    now: LogicalTime(0),
};
let events = table.handle(&Command::Roll, &mut ctx).unwrap();
```

## Hidden information (feature `redaction`)

`#[derive(Redact)]` generates a per-viewer view: the viewer sees their own
hidden values in full, everyone else sees only the public residue, and the view
type *cannot even represent* another principal's hidden data.

```rust,ignore
#[derive(Redact, StableHash, Clone, Debug, PartialEq)]
#[redact(principal = ParticipantId)]
struct MatchState {
    phase: MatchPhase,                                   // public
    board: Board,                                        // public
    #[hidden]          hands: PerPrincipal<ParticipantId, Hand>,   // owner full, others a count
    #[hidden]          fabrication: Owned<ParticipantId, FabricationId>,
    #[hidden(conceal)] decks: PerPrincipal<ParticipantId, Deck>,   // everyone sees only a count
}

let view = match_state.view_for(&me); // an MatchStateView with only what `me` may see
```

## Verifying aggregates (feature `proptest`)

```rust,ignore
ironstate_aggregate::test!(MatchState, cases = 1000, max_steps = 80);
ironstate_aggregate::determinism_test!(MatchState);            // two seeded runs must agree
ironstate_aggregate::leak_test!(MatchState, excluding = [PlayCard]); // no covert hidden→view flow
```

## A custom entropy source

Most consumers use the provided `SeededEntropy`. If you implement your own
`EntropySource` — to wrap a different generator, or to keep a stream your golden
vectors are already pinned to — verify it against the determinism contract in one
call:

```rust,ignore
ironstate_aggregate::assert_entropy_contract(|| MySource::from_seed(&seed));
```

It proves the unbiased-draw, O(1)-seek, and pure-probe properties the family's
replay guarantee rests on. Keep a separate golden-vector test to pin your
stream's exact bytes — the contract holds for any correct algorithm, not just
yours.

## Features

| Feature | Adds |
|---------|------|
| `derive` (default) | the `Redact`/`StableHash`/`Versioned` derives |
| `restore` (default) | versioned-event restore: decode a stored event and migrate it forward (pulls in serde) |
| `redaction` | `Conceal`/`OwnerRedact`/`PerPrincipal`/`Owned`, views, `leak_test!` |
| `stablehash` | the canonical encoding + `Digest128` |
| `audit` | `AuditDigest` (BLAKE3); implies `stablehash` |
| `proptest` | `test!`/`determinism_test!`/`leak_test!`; implies `stablehash` |
| `serde` | `Serialize`/`Deserialize` on `DrawPos`/`LogicalTime` |

## Learn more

New here? The [guide](https://github.com/kassian-dev/ironstate/blob/main/docs/guide.md) is a step-by-step walkthrough from a first machine through aggregates and the event journal. For complete runnable programs, see the [`ledger`](https://github.com/kassian-dev/ironstate/tree/main/app/crates/examples/ledger) example (`decide`/`evolve` with a non-negative-balance invariant) and [`hidden-info`](https://github.com/kassian-dev/ironstate/tree/main/app/crates/examples/hidden-info) (redaction end to end). For what each test layer proves, see [docs/testing.md](https://github.com/kassian-dev/ironstate/blob/main/docs/testing.md).

## The ironstate family

- [`ironstate`](https://crates.io/crates/ironstate) — core lifecycle machines
- [`ironstate-aggregate`](https://crates.io/crates/ironstate-aggregate) — deterministic aggregates (this crate)
- [`ironstate-journal`](https://crates.io/crates/ironstate-journal) — the event journal that persists and replays the events aggregates emit

Licensed under Apache-2.0.
