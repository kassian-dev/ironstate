# hidden-info

Worked hidden-information example — the family's integration template.

A tiny two-player card match that exercises the whole stack: a phase machine,
a `decide`/`evolve` aggregate that draws entropy, redaction (`#[hidden]`
hands, an `Owned` secret, a `#[hidden(conceal)]` deck), a memory journal
driven by `execute`/`resume`, a system-kind timeout command minted by the
embedding layer, a `Subscription` to a second aggregate, and all five test
macros. It must build in CI — it is the anti-drift gate for the family.

## Run it

```sh
# cargo commands run from app/
cargo run -p hidden-info    # the demo
cargo test -p hidden-info   # the end-to-end test module
```
