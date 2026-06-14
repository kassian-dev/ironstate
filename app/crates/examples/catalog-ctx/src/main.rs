#![doc = include_str!("../README.md")]

use std::sync::Arc;

use anyhow::{Result, anyhow};
use ironstate::prelude::*;
use ironstate_aggregate::{
    Aggregate, AggregateArbitrary, AggregateInvariant, AggregateInvariants, AggregateRules,
    CtxEntropy, EntropySource, LogicalTime, Seed, SeededEntropy, StableHash,
};
use ironstate_journal::{ExecuteError, MemoryJournal, execute, resume};
use proptest::prelude::*;

/// How many draws an opened chest yields before it must be closed.
const CAPACITY: u32 = 8;

// --- the catalog: read-only reference data --------------------------------

/// One weighted entry in the loot table.
struct Entry {
    item: u16,
    name: &'static str,
    /// Relative draw weight; the chance of this item is `weight / total`.
    weight: u64,
    /// The single rarest item — the only one a `Gamble` pays out on.
    jackpot: bool,
}

/// The loot table: fixed reference data, never part of aggregate state. It is
/// what an engine would otherwise thread by `&`/`&mut`; here it rides inside the
/// owned context by `Arc`, so `decide` can read it without a lifetime.
struct Catalog {
    entries: Vec<Entry>,
    total: u64,
}

impl Catalog {
    /// The standard loot table. Construction is cheap, so a fresh one per turn is
    /// fine; share a single `Arc` if it is ever expensive to build.
    fn standard() -> Self {
        let entries = vec![
            Entry {
                item: 1,
                name: "copper coin",
                weight: 60,
                jackpot: false,
            },
            Entry {
                item: 2,
                name: "silver ring",
                weight: 25,
                jackpot: false,
            },
            Entry {
                item: 3,
                name: "gold idol",
                weight: 10,
                jackpot: false,
            },
            Entry {
                item: 4,
                name: "dragon's eye",
                weight: 4,
                jackpot: false,
            },
            Entry {
                item: 5,
                name: "worldheart gem",
                weight: 1,
                jackpot: true,
            },
        ];
        let total = entries.iter().map(|e| e.weight).sum();
        Self { entries, total }
    }

    /// The total weight — the half-open range a draw samples over.
    fn total_weight(&self) -> u64 {
        self.total
    }

    /// The item whose cumulative weight bucket contains `pick` (in
    /// `0..total_weight`). Walking the cumulative weights keeps the mapping a pure
    /// function of the drawn word, so replay reproduces it exactly.
    fn item_at(&self, pick: u64) -> u16 {
        let mut acc = 0;
        for entry in &self.entries {
            acc += entry.weight;
            if pick < acc {
                return entry.item;
            }
        }
        // `pick < total_weight` always falls in a bucket above; the last entry is
        // the saturating fallback only if a caller passes an out-of-range value.
        self.entries.last().expect("a non-empty catalog").item
    }

    /// Whether `item` is the jackpot.
    fn is_jackpot(&self, item: u16) -> bool {
        self.entries.iter().any(|e| e.item == item && e.jackpot)
    }

    /// The display name of an item id.
    fn name(&self, item: u16) -> &'static str {
        self.entries
            .iter()
            .find(|e| e.item == item)
            .map_or("unknown", |e| e.name)
    }
}

/// Draw one item from the catalog, consuming exactly the entropy the mapping
/// needs. The only place this engine draws.
fn draw_item(catalog: &Catalog, entropy: &mut dyn EntropySource) -> u16 {
    let pick = entropy.draw_range(0..catalog.total_weight());
    catalog.item_at(pick)
}

// --- the owned context (the centerpiece) ----------------------------------

/// The owned form of a borrowing `TurnCtx<'a>`.
///
/// The catalog rides by `Arc` and the live entropy by `Box` — both owned, so the
/// whole context satisfies ironstate's owned `type Ctx` with no lifetime. One of
/// these is built once per turn and reused across commands; the engine never
/// threads it and never rewinds it.
struct TurnCtx {
    /// Read-only reference data, shared cheaply.
    catalog: Arc<Catalog>,
    /// The live, journaled entropy stream. Owned, but still O(1)-seekable, so
    /// `execute` can rewind it after a failure.
    entropy: Box<dyn EntropySource>,
    /// Who is taking the turn.
    actor: u32,
    /// Logical time, as data.
    now: LogicalTime,
}

/// The one bridge the persistent loop needs: reach the entropy inside the opaque
/// context so `execute` can read its post-decide position and rewind it. This is
/// the line that replaces every by-hand `entropy.seek(..)` the old engine ran.
impl CtxEntropy for TurnCtx {
    fn entropy_mut(&mut self) -> Option<&mut dyn EntropySource> {
        Some(&mut *self.entropy)
    }
}

// --- the phase machine ----------------------------------------------------

#[derive(StateMachine, StableHash, Clone, Debug, PartialEq)]
#[state_machine(initial = Sealed, terminal = [Spent])]
enum ChestPhase {
    Sealed,
    #[only_accepts(kind = "loot")]
    Open,
    Spent,
}

#[derive(Event, Clone, Debug, PartialEq)]
enum PhaseStep {
    Unseal,
    Exhaust,
}

impl TransitionRules for ChestPhase {
    type Event = PhaseStep;
    fn transition(&self, step: &PhaseStep) -> Option<ChestPhase> {
        use ChestPhase::*;
        use PhaseStep::*;
        match (self, step) {
            (Sealed, Unseal) => Some(Open),
            (Open, Exhaust) => Some(Spent),
            _ => None,
        }
    }
}

// --- commands, events, errors ---------------------------------------------

#[derive(Event, Clone, Debug, PartialEq)]
enum Command {
    Open,
    #[event_kind = "loot"]
    Draw,
    #[event_kind = "loot"]
    Gamble,
    #[event_kind = "loot"]
    Close,
}

#[derive(Clone, Debug, PartialEq)]
enum ChestEvent {
    Opened,
    Dropped { item: u16 },
    Closed,
}

#[derive(Debug, thiserror::Error)]
enum ChestError {
    #[error("the chest is already open")]
    NotSealed,
    #[error("the chest is not open")]
    NotOpen,
    #[error("no draws remain; close the chest")]
    Empty,
    #[error("the gamble drew a common, not the jackpot")]
    GambleLost,
}

// --- the aggregate --------------------------------------------------------

#[derive(StableHash, Clone, Debug, PartialEq)]
struct Chest {
    phase: ChestPhase,
    draws_left: u32,
    loot: Vec<u16>,
}

impl Chest {
    fn sealed() -> Self {
        Self {
            phase: ChestPhase::Sealed,
            draws_left: 0,
            loot: Vec::new(),
        }
    }
}

impl AggregateRules for Chest {
    type Phase = ChestPhase;
    type Command = Command;
    type Event = ChestEvent;
    type Error = ChestError;
    type Ctx = TurnCtx;

    fn phase(&self) -> ChestPhase {
        self.phase.clone()
    }

    fn decide(&self, cmd: &Command, ctx: &mut TurnCtx) -> Result<Vec<ChestEvent>, ChestError> {
        match cmd {
            Command::Open => {
                if self.phase != ChestPhase::Sealed {
                    return Err(ChestError::NotSealed);
                }
                Ok(vec![ChestEvent::Opened])
            }
            Command::Draw => {
                if self.phase != ChestPhase::Open {
                    return Err(ChestError::NotOpen);
                }
                if self.draws_left == 0 {
                    return Err(ChestError::Empty);
                }
                // Reads the catalog and draws entropy in the same step — both come
                // from the owned context, neither threaded as an argument.
                let item = draw_item(&ctx.catalog, &mut *ctx.entropy);
                Ok(vec![ChestEvent::Dropped { item }])
            }
            Command::Gamble => {
                if self.phase != ChestPhase::Open {
                    return Err(ChestError::NotOpen);
                }
                if self.draws_left == 0 {
                    return Err(ChestError::Empty);
                }
                // The recollect-shaped case: `decide` draws *before* the rule that
                // may reject it. A losing gamble has already advanced the live
                // stream — but `execute` rewinds it, so the draw costs nothing and
                // the next command sees the very same word. (The flip side: a
                // losing gamble retried from the same position loses identically;
                // determinism, not a bug.)
                let item = draw_item(&ctx.catalog, &mut *ctx.entropy);
                if ctx.catalog.is_jackpot(item) {
                    Ok(vec![ChestEvent::Dropped { item }])
                } else {
                    Err(ChestError::GambleLost)
                }
            }
            Command::Close => {
                if self.phase != ChestPhase::Open {
                    return Err(ChestError::NotOpen);
                }
                Ok(vec![ChestEvent::Closed])
            }
        }
    }

    fn evolve(&mut self, event: &ChestEvent) {
        match event {
            ChestEvent::Opened => {
                self.phase = ChestPhase::Open;
                self.draws_left = CAPACITY;
            }
            ChestEvent::Dropped { item } => {
                self.loot.push(*item);
                self.draws_left = self.draws_left.saturating_sub(1);
            }
            ChestEvent::Closed => self.phase = ChestPhase::Spent,
        }
    }
}

impl AggregateInvariants for Chest {
    fn invariants() -> Vec<AggregateInvariant<Self>> {
        vec![
            // Every drop adds exactly one item and spends exactly one draw — the
            // catalog read and the entropy draw stay in lockstep with the state.
            AggregateInvariant::<Self>::custom("a drop adds one item and spends one draw").assert(
                |before, event, after| match event {
                    ChestEvent::Dropped { .. } => {
                        after.loot.len() == before.loot.len() + 1
                            && after.draws_left + 1 == before.draws_left
                    }
                    _ => true,
                },
            ),
        ]
    }
}

impl AggregateArbitrary for Chest {
    fn initial_state_strategy() -> BoxedStrategy<Self> {
        Just(Chest::sealed()).boxed()
    }

    fn command_strategy(_state: &Self) -> BoxedStrategy<Command> {
        // Phase-agnostic: decide and the phase machine reject what does not fit
        // the current phase, and the test drivers record and skip those.
        prop_oneof![
            1 => Just(Command::Open),
            4 => Just(Command::Draw),
            2 => Just(Command::Gamble),
            1 => Just(Command::Close),
        ]
        .boxed()
    }

    fn test_ctx(entropy: Box<dyn EntropySource>, step: u64) -> Self::Ctx {
        // The suites take ownership of the entropy and reach it back through
        // `CtxEntropy` — the same owned context the engine uses in production.
        TurnCtx {
            catalog: Arc::new(Catalog::standard()),
            entropy,
            actor: 0,
            now: LogicalTime(step),
        }
    }
}

// The same verification ladder every aggregate runs: structural enforcement and
// invariants, digest-for-digest determinism, and the seven-property journal
// contract — now exercising the owned catalog-bearing context.
ironstate_aggregate::test!(Chest, cases = 200, max_steps = 30);
ironstate_aggregate::determinism_test!(Chest, cases = 100, max_steps = 30);
ironstate_journal::journal_contract_test!(Chest);

// --- a walk-through that proves the rewind is automatic --------------------

fn step(
    journal: &mut MemoryJournal<Chest>,
    chest: &mut Aggregate<Chest>,
    cmd: &Command,
    ctx: &mut TurnCtx,
) -> Result<()> {
    execute(journal, chest, cmd, ctx)
        .map(|_| ())
        .map_err(|e| anyhow!("{e}"))
}

/// Open a chest and take `draws` draws from a fixed seed, returning the loot.
/// The control run: it never gambles.
fn straight_run(catalog: &Arc<Catalog>, seed: &Seed, draws: usize) -> Result<Vec<u16>> {
    let mut journal = MemoryJournal::new(Chest::sealed());
    let mut chest = Aggregate::new(Chest::sealed())?;
    let mut ctx = TurnCtx {
        catalog: catalog.clone(),
        entropy: Box::new(SeededEntropy::from_seed(seed)),
        actor: 1,
        now: LogicalTime(0),
    };
    step(&mut journal, &mut chest, &Command::Open, &mut ctx)?;
    for _ in 0..draws {
        step(&mut journal, &mut chest, &Command::Draw, &mut ctx)?;
    }
    Ok(chest.state().loot.clone())
}

fn run_demo() -> Result<()> {
    let catalog = Arc::new(Catalog::standard());
    let seed = Seed([7u8; 32]);

    let mut journal = MemoryJournal::new(Chest::sealed());
    let mut chest = Aggregate::new(Chest::sealed())?;

    // ONE owned context, one live stream, reused across the turn — the owned
    // mirror of a borrowed live stream threaded through the engine.
    let mut ctx = TurnCtx {
        catalog: catalog.clone(),
        entropy: Box::new(SeededEntropy::from_seed(&seed)),
        actor: 1,
        now: LogicalTime(0),
    };
    println!(
        "turn taken by actor {} at logical time {:?}",
        ctx.actor, ctx.now
    );

    step(&mut journal, &mut chest, &Command::Open, &mut ctx)?;
    step(&mut journal, &mut chest, &Command::Draw, &mut ctx)?;
    step(&mut journal, &mut chest, &Command::Draw, &mut ctx)?;
    println!(
        "two draws: {}",
        chest
            .state()
            .loot
            .iter()
            .map(|i| catalog.name(*i))
            .collect::<Vec<_>>()
            .join(", ")
    );

    // The live stream sits exactly here after two real draws.
    let before_gamble = ctx.entropy.draws();

    // A gamble draws from the same stream, then the domain rule rejects it (a
    // common, not the jackpot). We do *not* rewind by hand.
    let lost = execute(&mut journal, &mut chest, &Command::Gamble, &mut ctx);
    assert!(
        matches!(lost, Err(ExecuteError::Rejected(_))),
        "the gamble should lose on this seed"
    );

    // The headline: `execute` rewound the owned stream. The losing gamble cost no
    // entropy, mutated no state, and journaled nothing.
    assert_eq!(ctx.entropy.draws(), before_gamble, "the stream was rewound");
    println!("gamble lost; live stream rewound to position {before_gamble:?}");

    // Proof the rewind is exact, not approximate: the next real draw yields what a
    // control run that never gambled gets for its third draw. If the gamble's draw
    // had leaked through, this item would differ.
    step(&mut journal, &mut chest, &Command::Draw, &mut ctx)?;
    let after_gamble_item = *chest.state().loot.last().expect("a third drop");
    let control = straight_run(&catalog, &seed, 3)?;
    assert_eq!(
        after_gamble_item, control[2],
        "the draw after a losing gamble matches the gamble-free control",
    );
    println!(
        "third draw after the lost gamble: {} (== control's third draw)",
        catalog.name(after_gamble_item)
    );

    // And the whole run rebuilds from the log alone — the owned context changed
    // nothing about determinism.
    let (resumed, _entropy) = resume::<Chest, _>(&journal, &seed).map_err(|e| anyhow!("{e}"))?;
    assert_eq!(
        resumed.state().loot,
        chest.state().loot,
        "resume reproduces the loot"
    );
    println!("resumed from the log; loot matches");

    Ok(())
}

fn main() -> Result<()> {
    run_demo()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironstate_aggregate::Rejection;

    #[test]
    fn demo_runs() {
        run_demo().unwrap();
    }

    #[test]
    fn a_losing_gamble_rewinds_the_live_stream() {
        // Reproduce the rewind in isolation: a gamble that draws then is rejected
        // must leave the owned stream exactly where it started.
        let catalog = Arc::new(Catalog::standard());
        let seed = Seed([7u8; 32]);
        let mut journal = MemoryJournal::new(Chest::sealed());
        let mut chest = Aggregate::new(Chest::sealed()).unwrap();
        let mut ctx = TurnCtx {
            catalog,
            entropy: Box::new(SeededEntropy::from_seed(&seed)),
            actor: 1,
            now: LogicalTime(0),
        };

        step(&mut journal, &mut chest, &Command::Open, &mut ctx).unwrap();
        let before = ctx.entropy.draws();
        let head_before = chest.state().clone();

        let lost = execute(&mut journal, &mut chest, &Command::Gamble, &mut ctx);
        assert!(matches!(lost, Err(ExecuteError::Rejected(_))));

        // Entropy rewound, state untouched, nothing journaled.
        assert_eq!(ctx.entropy.draws(), before);
        assert_eq!(chest.state(), &head_before);
    }

    #[test]
    fn closed_chest_is_terminal() {
        let mut chest = Aggregate::new(Chest::sealed()).unwrap();
        let mut ctx = TurnCtx {
            catalog: Arc::new(Catalog::standard()),
            entropy: Box::new(SeededEntropy::from_seed(&Seed([0; 32]))),
            actor: 1,
            now: LogicalTime(0),
        };
        chest.handle(&Command::Open, &mut ctx).unwrap();
        chest.handle(&Command::Close, &mut ctx).unwrap();
        assert_eq!(chest.phase(), ChestPhase::Spent);

        let rejection = chest.handle(&Command::Draw, &mut ctx).unwrap_err();
        assert!(matches!(rejection, Rejection::TerminalPhase { .. }));
    }
}
