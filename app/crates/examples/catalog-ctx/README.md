# catalog-ctx

Adopting ironstate's owned `type Ctx` for an engine that used to thread a
borrowing context.

Many engines carry a *borrowing* turn context — a `TurnCtx<'a>` holding
reference data (a catalog) and the live entropy stream by `&mut`, threaded
through every engine function, with the live stream rewound by hand when a
turn is rejected. ironstate's `AggregateRules::Ctx` is a plain associated
type, so it cannot name a caller-introduced lifetime: the context the runtime
threads must be **owned**. That looks like a loss; it is not. Two moves close
the gap, and one of them is a net simplification:

- **Catalog → `Arc`.** Read-only reference data goes into the owned context by
  `Arc` (or `&'static`, or `Rc` single-threaded). Cheap to clone, no lifetime,
  and `decide` stays `&self`/non-mutating — it only *reads* the catalog.
- **Live entropy → `Box`.** The stream goes in by `Box<dyn EntropySource>`.
  Owning it costs nothing, because a `SeededEntropy` reseeks to any position in
  O(1) from `(seed, position)`. So the by-hand rewind disappears entirely:
  `execute` rewinds the owned stream for you on any rejected or failed command.

The result is one owned context, reused across the turn, that the engine no
longer has to thread — and a rewind it no longer has to perform. This example
is a loot chest: `decide` reads the catalog *and* draws entropy in the same
step, exactly the shape that made the borrowing context awkward. It runs the
same determinism and journal-contract suites every other aggregate does, so
the owned context is held to the identical bar.

## Run it

```sh
# cargo commands run from app/
cargo run -p catalog-ctx    # the demo
cargo test -p catalog-ctx   # the end-to-end test module
```
