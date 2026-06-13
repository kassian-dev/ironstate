//! A well-behaved aggregate that passes `test!` and `determinism_test!`,
//! exercising the AggregateArbitrary driver, declared invariants, and the
//! two-run determinism check.
#![cfg(feature = "proptest")]

use ironstate::prelude::*;
use ironstate_aggregate::{
    Aggregate, AggregateArbitrary, AggregateInvariant, AggregateInvariants, AggregateRules,
    DrawPos, EntropySource, LogicalTime, OwnedDeterministicCtx, Seed, SeededEntropy, StableHash,
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
    fn transition(&self, step: &Step) -> Option<Phase> {
        match (self, step) {
            (Phase::Open, Step::Close) => Some(Phase::Closed),
            _ => None,
        }
    }
}

#[derive(Event, Clone, Debug, PartialEq)]
enum Command {
    Tick,
    Close,
}

#[derive(Clone, Debug, PartialEq)]
enum CounterEvent {
    Ticked(u8),
    Closed,
}

#[derive(Debug, thiserror::Error)]
#[error("the counter is closed")]
struct Closed;

#[derive(StableHash, Clone, Debug, PartialEq)]
struct Counter {
    phase: Phase,
    sum: u32,
    ticks: u32,
}

impl AggregateRules for Counter {
    type Phase = Phase;
    type Command = Command;
    type Event = CounterEvent;
    type Error = Closed;
    type Ctx = OwnedDeterministicCtx<u32>;

    fn phase(&self) -> Phase {
        self.phase.clone()
    }

    fn decide(&self, cmd: &Command, ctx: &mut Self::Ctx) -> Result<Vec<CounterEvent>, Closed> {
        if self.phase != Phase::Open {
            return Err(Closed);
        }
        match cmd {
            Command::Tick => Ok(vec![CounterEvent::Ticked(
                ctx.entropy.draw_range(0..10) as u8
            )]),
            Command::Close => Ok(vec![CounterEvent::Closed]),
        }
    }

    fn evolve(&mut self, event: &CounterEvent) {
        match event {
            CounterEvent::Ticked(n) => {
                self.sum += u32::from(*n);
                self.ticks += 1;
            }
            CounterEvent::Closed => self.phase = Phase::Closed,
        }
    }
}

impl AggregateInvariants for Counter {
    fn invariants() -> Vec<AggregateInvariant<Self>> {
        vec![
            AggregateInvariant::<Self>::custom("the running sum never decreases")
                .assert(|before, _event, after| after.sum >= before.sum),
        ]
    }
}

impl AggregateArbitrary for Counter {
    fn initial_state_strategy() -> BoxedStrategy<Self> {
        Just(Counter {
            phase: Phase::Open,
            sum: 0,
            ticks: 0,
        })
        .boxed()
    }

    fn command_strategy(_state: &Self) -> BoxedStrategy<Command> {
        prop_oneof![5 => Just(Command::Tick), 1 => Just(Command::Close)].boxed()
    }

    fn test_ctx(entropy: Box<dyn EntropySource>, step: u64) -> Self::Ctx {
        OwnedDeterministicCtx {
            entropy,
            actor: 0,
            now: LogicalTime(step),
        }
    }
}

ironstate_aggregate::test!(Counter, cases = 100, max_steps = 30);
ironstate_aggregate::determinism_test!(Counter, cases = 50, max_steps = 30);

#[test]
fn a_concrete_run_is_reproducible() {
    // Same seed and position give the same dice, so two hand-run sequences match.
    let run = |seed: u64| {
        let mut agg = Aggregate::new(Counter {
            phase: Phase::Open,
            sum: 0,
            ticks: 0,
        })
        .unwrap();
        let mut pos = DrawPos(0);
        for step in 0..5u64 {
            let mut ctx = OwnedDeterministicCtx {
                entropy: Box::new(SeededEntropy::at(&Seed([seed as u8; 32]), pos)),
                actor: 0,
                now: LogicalTime(step),
            };
            agg.handle(&Command::Tick, &mut ctx).unwrap();
            pos = ctx.entropy.draws();
        }
        agg.state().clone()
    };
    assert_eq!(run(3), run(3));
}
