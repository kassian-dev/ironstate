//! The reference `MemoryJournal` must pass the seven-property conformance suite.
#![cfg(feature = "sim")]

use ironstate::prelude::*;
use ironstate_aggregate::{
    AggregateArbitrary, AggregateRules, EntropySource, LogicalTime, OwnedDeterministicCtx,
    StableHash,
};
use proptest::prelude::*;

#[derive(StateMachine, StableHash, Clone, Debug, PartialEq)]
#[state_machine(initial = Open, terminal = [Closed])]
enum Phase {
    Open,
    Closed,
}
#[derive(Event, Clone, Debug, PartialEq)]
enum Step {
    Close,
}
impl TransitionRules for Phase {
    type Event = Step;
    fn transition(&self, _: &Step) -> Option<Phase> {
        matches!(self, Phase::Open).then_some(Phase::Closed)
    }
}

#[derive(Event, Clone, Debug, PartialEq)]
enum Command {
    Tick,
    Close,
}
#[derive(Clone, Debug, PartialEq)]
enum Ev {
    Rolled(u8),
    Closed,
}
#[derive(Debug, thiserror::Error)]
#[error("closed")]
struct ClosedErr;

#[derive(StableHash, Clone, Debug, PartialEq)]
struct Counter {
    phase: Phase,
    total: u32,
}

impl AggregateRules for Counter {
    type Phase = Phase;
    type Command = Command;
    type Event = Ev;
    type Error = ClosedErr;
    type Ctx = OwnedDeterministicCtx<u32>;

    fn phase(&self) -> Phase {
        self.phase.clone()
    }
    fn decide(&self, cmd: &Command, ctx: &mut Self::Ctx) -> Result<Vec<Ev>, ClosedErr> {
        if self.phase != Phase::Open {
            return Err(ClosedErr);
        }
        Ok(match cmd {
            Command::Tick => vec![Ev::Rolled(ctx.entropy.draw_range(1..7) as u8)],
            Command::Close => vec![Ev::Closed],
        })
    }
    fn evolve(&mut self, event: &Ev) {
        match event {
            Ev::Rolled(n) => self.total += u32::from(*n),
            Ev::Closed => self.phase = Phase::Closed,
        }
    }
}

impl AggregateArbitrary for Counter {
    fn initial_state_strategy() -> BoxedStrategy<Self> {
        Just(Counter {
            phase: Phase::Open,
            total: 0,
        })
        .boxed()
    }
    fn command_strategy(_state: &Self) -> BoxedStrategy<Command> {
        // Mostly ticks (which draw entropy and advance positions), occasionally a close.
        prop_oneof![8 => Just(Command::Tick), 1 => Just(Command::Close)].boxed()
    }
    fn test_ctx(entropy: Box<dyn EntropySource>, step: u64) -> Self::Ctx {
        OwnedDeterministicCtx {
            entropy,
            actor: 0,
            now: LogicalTime(step),
        }
    }
}

ironstate_journal::journal_contract_test!(Counter);
ironstate_journal::scenario_test!(Counter, cases = 200, max_steps = 40);
