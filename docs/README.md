# ironstate documentation

Start with the guide and reach for the rest as you need it.

| Read | When |
|------|------|
| [guide.md](guide.md) | **First.** A step-by-step walkthrough, from a first state machine to a deterministic, event-sourced aggregate with an audit log. |
| [design.md](design.md) | To understand *why* the family is shaped this way: the two paradigms, the verification ladder, the determinism contract, and the vocabulary. |
| [testing.md](testing.md) | When you're ready to verify your own code: the test taxonomy, what each layer proves, and which macro to reach for. |
| [decisions/](decisions) | For the reasoning behind a specific choice — including what was deliberately deferred or dropped. |

For working code, the [examples](../app/crates/examples) are runnable programs that double as end-to-end tests; their [index](../app/crates/examples) maps each one to the concepts it shows. The API reference is on docs.rs, one page per crate: [ironstate](https://docs.rs/ironstate), [ironstate-aggregate](https://docs.rs/ironstate-aggregate), [ironstate-journal](https://docs.rs/ironstate-journal).
