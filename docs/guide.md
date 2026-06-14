# The ironstate guide

A step-by-step walkthrough, from your first state machine to a deterministic, event-sourced aggregate with an audit log. No prior background in formal methods or event sourcing is assumed — every term is introduced as it comes up.

If you'd rather read working code, the [`examples/`](../app/crates/examples) directory has runnable, fully-tested programs. Three of them mirror this guide: `release-pipeline` (a core machine), `ledger` (an aggregate over a journal), and `hidden-info` (redaction and the whole stack together).

## 1. What problem this solves

Almost every program has state that moves through stages. An order is placed, then paid, then shipped. A document is a draft, then in review, then published. A support ticket is open, then pending, then closed. The rules are simple — you must not ship before payment, you must not publish a draft — but in most code they're spread across `if`s and `match`es in a dozen places, and that's where the bugs live: a transition nobody guarded, a case nobody handled, the "this can't happen" that happens in production.

ironstate's bet is that if you write the states and the rules down *once*, in one place, you can both *enforce* them at runtime and *verify* them in tests from that single definition. You stop writing the rules twice (once in code, once in your test fixtures) and stop letting them drift apart.

## 2. Your first machine

Three pieces: an enum of states, an enum of events, and one function that says which event moves you from which state to which.

```rust
use ironstate::prelude::*;

// The states. `initial` is where a fresh machine starts; `terminal` states
// have no way out.
#[derive(StateMachine, Clone, Debug, PartialEq)]
#[state_machine(initial = Draft, terminal = [Archived])]
enum Article { Draft, Review, Published, Archived }

// The events that drive it.
#[derive(Event, Clone, Debug, PartialEq)]
enum Edit { Submit, Approve, Reject, Archive }

// The one rule function: return Some(next) for a legal move, None otherwise.
impl TransitionRules for Article {
    type Event = Edit;
    fn transition(&self, event: &Edit) -> Option<Article> {
        use Article::*;
        use Edit::*;
        match (self, event) {
            (Draft, Submit)      => Some(Review),
            (Review, Approve)    => Some(Published),
            (Review, Reject)     => Some(Draft),
            (Published, Archive) => Some(Archived),
            _ => None,
        }
    }
}
```

Now drive it. `apply` is the one call that changes state, and it always returns a `Result` — ironstate never panics on a bad move, it hands you a typed error.

```rust,ignore
let mut article = Machine::<Article>::new();           // Draft
article.apply(Edit::Submit)?;                          // Review
article.apply(Edit::Approve)?;                         // Published
```

Two structural guarantees come for free, checked before your `transition` function even runs: a terminal state rejects every event, and (next section) a state can restrict which *kinds* of event it accepts. These aren't tested properties — they hold by construction.

Three cheaper ways to look without moving, in increasing detail:

```rust,ignore
article.could_apply(&Edit::Approve);     // bool — would it be accepted?
article.why_not(&Edit::Approve);         // Option<TransitionError> — the exact reason it wouldn't
article.peek_transition(&Edit::Approve); // Option<Article> — where it would land
```

When `apply` rejects an event, the event isn't lost — it moves *into* the error, so you can retry it, log it, or route it elsewhere without cloning:

```rust,ignore
match article.apply(some_event) {
    Ok(state) => { /* moved */ }
    Err(e) => {
        let event = e.into_event(); // got it back
    }
}
```

## 3. Verifying it: the definition is the test

You wrote the machine; now check it, from the same definition.

`analyze!` walks the state graph and proves structural facts. Drop it in a test module and it becomes a `#[test]` that fails on real design errors — an unreachable state, a deadlock (a non-terminal state with no way out), an inescapable cycle, or a *dead transition* (one your function defines but the structure could never allow). It caught a real bug while this library was being built: a state marked terminal that still had an outgoing move.

```rust,ignore
#[cfg(test)]
ironstate::analyze!(Article);
```

`test!` generates thousands of random event sequences and checks, after every step, that your rules hold and nothing panicked. If you declare invariants — properties that must always be true — it checks those too. When something breaks, it shrinks to the shortest failing sequence and is reproducible with a seed.

```rust,ignore
impl Invariants for Article {
    fn invariants() -> Vec<Invariant<Self, Self::Event>> {
        vec![Invariant::custom("an archived article stays archived")
            .assert(|before, _event, after| {
                before != &Article::Archived || after.is_none()
            })]
    }
}

#[cfg(test)]
ironstate::test!(Article, cases = 1000, seed = 0xC0FFEE);
```

Every claim these print is labeled `[proven]` (holds by construction) or `[sampled]` (observed over generated input), so you always know how strong a statement is.

## 4. Event kinds: gating who can do what

Some states should only respond to certain *sources*. A deploy step waits on an external system; an approval step waits on a human operator. Mark the state with the kinds it accepts, and the events with the kind they carry — the runtime rejects a mismatch (with a typed `EventKindRejected`) before your function runs.

```rust,ignore
#[derive(StateMachine, Clone, Debug, PartialEq)]
#[state_machine(initial = Pending, terminal = [Done])]
enum Deploy {
    Pending,
    #[only_accepts(kind = "external")] // only the deploy target may move this
    Deploying,
    Done,
}

#[derive(Event, Clone, Debug, PartialEq)]
enum Signal {
    Start,
    #[event_kind = "external"] Complete,
}
```

The [`release-pipeline`](../app/crates/examples/release-pipeline) example uses this for operator- and external-gated states.

## 5. Persisting and upgrading state

States get saved to a database and loaded back, and your enum changes over time. Declare a `version` and the retired types, write a pure `migrate` for each step, and `restore_versioned` decodes an old `{version, payload}` envelope and walks it forward to today's shape. A version newer than your binary understands comes back as a typed error, never a panic.

```rust,ignore
#[derive(StateMachine, Serialize, Deserialize, Clone, Debug, PartialEq)]
#[state_machine(initial = Draft, terminal = [Retired], version = 3, history = [DocV1, DocV2])]
enum Doc { Draft, Live, Retired }

impl MigrateFrom<DocV1> for DocV2 { /* … */ }
impl MigrateFrom<DocV2> for Doc   { /* … */ }

let doc = Machine::<Doc>::restore_versioned(&bytes)?;
```

## 6. Aggregates: when one event isn't one hop

A core machine is "one event → one hop." But some things change through a *stream of facts* and a single command can produce several of them: a bank withdrawal checks the balance and emits a "withdrawn" fact; a game move might draw a card *and* end a turn. That's an **aggregate**, and it lives in [`ironstate-aggregate`](https://docs.rs/ironstate-aggregate).

An aggregate is split into two functions, and the split is the whole point:

- `decide(&self, command, ctx)` is where all your rules live. It validates a command (intent, which may be rejected) and returns the events (facts, which already happened) that should follow. It's the *only* place randomness is drawn. It does not change state.
- `evolve(&mut self, event)` applies one fact to the state. It is total and pure: it never fails, never draws randomness, never reads a clock.

Because `evolve` is pure, replaying the same events always rebuilds the same state. That's what makes the next two sections possible.

```rust,ignore
impl AggregateRules for Account {
    type Phase = AccountPhase;  // a core StateMachine for the lifecycle
    type Command = Command;     // intent: Deposit, Withdraw, …
    type Event = LedgerEvent;   // facts: Deposited, Withdrawn, …
    type Error = LedgerError;
    type Ctx = OwnedDeterministicCtx<AccountId>;

    fn phase(&self) -> AccountPhase { self.phase.clone() }

    fn decide(&self, cmd: &Command, _ctx: &mut Self::Ctx) -> Result<Vec<LedgerEvent>, LedgerError> {
        match cmd {
            Command::Withdraw { cents } if *cents > self.balance => Err(LedgerError::Overdraft),
            Command::Withdraw { cents } => Ok(vec![LedgerEvent::Withdrawn { cents: *cents }]),
            // …
        }
    }

    fn evolve(&mut self, event: &LedgerEvent) {
        match event {
            LedgerEvent::Withdrawn { cents } => self.balance -= cents,
            // …
        }
    }
}
```

The `Ctx` is how `decide` reaches the read-only data and the randomness it needs — a product catalog, configuration, the random stream. It's an *owned* associated type (it can't borrow a caller's lifetime), so reference data goes in by `Arc`; the [`catalog-ctx`](../app/crates/examples/catalog-ctx) example shows that pattern in full.

`handle` runs the everyday loop in memory — structural checks, then `decide`, then `evolve` each event — and `why_not` tells you why a command would be rejected without running it. The [`ledger`](../app/crates/examples/ledger) example is a complete account aggregate.

### Randomness that replays

If `decide` needs randomness (shuffle a deck, roll a die), it draws from a journal-owned source addressed by a counter, not the OS. The same seed and position always produce the same draw, so a replay reproduces the same events exactly. The API deliberately has no float draw and no clock — it can't express an input that would break replay.

## 7. The journal: the log of what happened

[`ironstate-journal`](https://docs.rs/ironstate-journal) stores an aggregate's events so it can be rebuilt, branched, and audited.

`execute` is the durable version of `handle`: it checks the command, appends the events *and the random-stream position* to the log atomically, then applies them — so the log and memory never disagree, even if the process dies mid-step. `resume` rebuilds an aggregate from the log after a restart, and `replay_hash` produces a tamper-evident digest of the final state anyone holding the events can recompute.

```rust,ignore
let seq = execute(&mut journal, &mut account, &command, &mut ctx)?;
// …later, after a restart:
let (account, entropy) = resume(&journal, &seed)?;
```

The one subtlety: replaying events draws no randomness (only `decide` does), so the stream position can't be recomputed from the events — the journal records it with every append. Get this wrong in a storage adapter and replays drift; `journal_contract_test!` exists to catch exactly that, and any adapter you write is measured against it.

## 8. Hidden information

In a card game, each player sees their own hand but only the *size* of everyone else's. ironstate makes that a property of the type system, not a runtime check you might forget.

You say what the public "residue" of a secret is with `Conceal`, mark fields `#[hidden]`, and `view_for(player)` produces a view where that player sees their own values in full and everyone else sees only the residue. The view type literally cannot hold another player's hidden value.

```rust,ignore
#[derive(Redact, StableHash, Clone, Debug, PartialEq)]
#[redact(principal = PlayerId)]
struct Match {
    board: Vec<Card>,                              // public
    #[hidden] hands: PerPrincipal<PlayerId, Hand>, // owner sees cards, others a count
}

let what_alice_sees = game.view_for(&alice);
```

The [`hidden-info`](../app/crates/examples/hidden-info) example is a full match with hands, a per-owner secret, a shared deck, and a subscription to a second aggregate.

## 9. Testing the hard things

The same "definition is the test" idea covers the properties that are usually hardest to pin down. Each one applies to a specific shape of code — you reach for it when your type matches, not by default:

- **If your aggregate draws randomness** (or you rely on replay or audit digests), add `determinism_test!(MatchState)`: it runs the aggregate twice from the same seed and fails if the two runs ever disagree, byte-for-byte — catching sneaky non-determinism like iterating a `HashMap` inside `decide`.
- **If you have `#[hidden]` state** (per-viewer secrets), add `leak_test!(MatchState, excluding = [PlayCard])`: it checks that nothing one player keeps secret ever shows up in another player's view, across every operation except the ones that legitimately reveal information (which you list).
- **If you write your own storage adapter**, add `journal_contract_test!(MatchState)`: it holds the adapter to seven properties (round-trip, position discipline, crash atomicity, …). If you use the built-in journal, you already have this.
- **If durability under faults matters**, add `scenario_test!(MatchState)`: it drives the whole thing under a seeded storm of injected faults — failed appends, crashes-and-resumes, forks — and checks the faults are invisible to the final outcome.

For the full test taxonomy — including the CI-only fuzz and mutation layers — and a table of when to reach for each macro, see [`testing.md`](testing.md).

Determinism isn't a promise in prose; it's enforced. The `StableHash` derive refuses to compile a state that contains a float, a hash map, or a wall clock (each with a message telling you the fix), the entropy API has no way to read the clock, and `determinism_test!` fails the moment a run diverges.

## Where to go next

- The runnable examples: [`release-pipeline`](../app/crates/examples/release-pipeline), [`ledger`](../app/crates/examples/ledger), [`hidden-info`](../app/crates/examples/hidden-info).
- The API reference on docs.rs: [`ironstate`](https://docs.rs/ironstate), [`ironstate-aggregate`](https://docs.rs/ironstate-aggregate), [`ironstate-journal`](https://docs.rs/ironstate-journal).
- The design in one page: [`design.md`](design.md). Why the code is shaped as it is: [`decisions/`](decisions).
