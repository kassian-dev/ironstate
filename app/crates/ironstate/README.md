<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/kassian-dev/ironstate/main/assets/wordmark-dark.png">
    <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/kassian-dev/ironstate/main/assets/wordmark-light.png">
    <img src="https://raw.githubusercontent.com/kassian-dev/ironstate/main/assets/wordmark.png" alt="ironstate" width="320">
  </picture>
</p>

<p align="center"><strong>Verified state machines for humans and AI agents.</strong></p>

A state machine is the set of states something can be in and the legal moves between them. ironstate lets you declare those states and one transition function in plain Rust, then enforces the structure at runtime and verifies it in your tests. The same definition does both jobs, so there is no separate model to keep in sync.

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

let mut article = Machine::<Article>::new();      // starts in Draft
assert_eq!(article.apply(Edit::Submit).unwrap(), Article::Review);

// Look before you leap, cheapest first:
assert!(article.could_apply(&Edit::Approve));     // would it be accepted?
assert!(article.why_not(&Edit::Submit).is_some()); // the typed reason it would not, or None
assert_eq!(article.peek_transition(&Edit::Approve), Some(Article::Published));

// A rejected event is handed back inside the error, so you can retry or re-route it:
let err = Machine::<Article>::restore(Article::Archived).apply(Edit::Submit).unwrap_err();
assert!(matches!(err, TransitionError::TerminalState { .. }));
assert_eq!(err.into_event(), Edit::Submit);
```

Verification is part of the API. Drop these in a test module and the definition tests itself:

```rust,ignore
ironstate::analyze!(Article);              // graph analysis: dead ends, unreachable states, dead transitions
ironstate::test!(Article, cases = 1000);   // randomized property testing against your declared invariants
```

## Cargo features

All three are on by default.

| Feature | Adds |
|---------|------|
| `derive` | the `StateMachine`/`Event` derive macros |
| `proptest` | the `test!` property-testing macro (`analyze!` is always available) |
| `restore` | versioned restore: decode a stored `{version, payload}` envelope and migrate it forward (pulls in serde) |

Building a plain in-memory machine with no serialization dependency? Set `default-features = false` and add back just `derive`.

## Learn more

New here? The [guide](https://github.com/kassian-dev/ironstate/blob/main/docs/guide.md) is a step-by-step walkthrough from a first machine through aggregates and the event journal. For a complete runnable program, see the [`release-pipeline`](https://github.com/kassian-dev/ironstate/tree/main/app/crates/examples/release-pipeline) example (operator/external-gated states, declared invariants, a pure transition function); the [full example set](https://github.com/kassian-dev/ironstate/tree/main/app/crates/examples) goes on through aggregates and the journal. For what each test layer proves, see [docs/testing.md](https://github.com/kassian-dev/ironstate/blob/main/docs/testing.md).

## The ironstate family

- [`ironstate`](https://crates.io/crates/ironstate) — core lifecycle machines (this crate)
- [`ironstate-aggregate`](https://crates.io/crates/ironstate-aggregate) — deterministic aggregates: decide/evolve, redaction, StableHash
- [`ironstate-journal`](https://crates.io/crates/ironstate-journal) — the event journal: append/replay/fork/subscribe

Licensed under Apache-2.0.
