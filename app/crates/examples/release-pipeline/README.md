# release-pipeline

A CI/CD release modeled as a core ironstate lifecycle machine.

It shows the core crate end to end: a state enum with operator- and
external-gated states, a pure transition function, declared invariants, and
the `analyze!`/`test!` verification macros. The build pipeline and the
deploy target drive the machine through events; nothing reads a clock.

## Run it

```sh
# cargo commands run from app/
cargo run -p release-pipeline    # the demo
cargo test -p release-pipeline   # the end-to-end test module
```
