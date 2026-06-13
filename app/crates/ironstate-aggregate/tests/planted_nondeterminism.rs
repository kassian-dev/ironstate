//! Test-the-testers: a deliberately nondeterministic aggregate that
//! `determinism_test!`'s driver MUST catch. If this stops panicking, the
//! determinism check has regressed.
#![cfg(feature = "proptest")]

use ironstate::prelude::*;
use ironstate_aggregate::testkit_support::{DriveParams, run_determinism};
use ironstate_aggregate::{
    AggregateArbitrary, AggregateRules, EntropySource, LogicalTime, OwnedDeterministicCtx,
    StableHash,
};
use proptest::prelude::*;
use std::collections::HashMap;

#[derive(StateMachine, StableHash, Clone, Debug, PartialEq)]
#[state_machine(initial = Open, terminal = [Done])]
enum Phase {
    Open,
    Done,
}

#[derive(Event, Clone, Debug, PartialEq)]
enum Step {
    Finish,
}

impl TransitionRules for Phase {
    type Event = Step;
    fn transition(&self, step: &Step) -> Option<Phase> {
        match (self, step) {
            (Phase::Open, Step::Finish) => Some(Phase::Done),
            _ => None,
        }
    }
}

#[derive(Event, Clone, Debug, PartialEq)]
enum Command {
    Go,
}

#[derive(Debug, thiserror::Error)]
#[error("never")]
struct Never;

#[derive(StableHash, Clone, Debug, PartialEq)]
struct Leaky {
    phase: Phase,
    log: Vec<u8>,
}

impl AggregateRules for Leaky {
    type Phase = Phase;
    type Command = Command;
    type Event = PushedEvent;
    type Error = Never;
    type Ctx = OwnedDeterministicCtx<u32>;

    fn phase(&self) -> Phase {
        self.phase.clone()
    }

    fn decide(&self, _cmd: &Command, _ctx: &mut Self::Ctx) -> Result<Vec<PushedEvent>, Never> {
        // THE PLANTED DEFECT: events are emitted in HashMap iteration order,
        // which is randomized per map instance — so the same command from the
        // same state produces different event orders across runs.
        let mut map: HashMap<u8, ()> = HashMap::new();
        for k in 1..=6u8 {
            map.insert(k, ());
        }
        Ok(map.keys().map(|k| PushedEvent(*k)).collect())
    }

    fn evolve(&mut self, event: &PushedEvent) {
        // Order-sensitive, so a different event order yields a different state.
        self.log.push(event.0);
    }
}

#[derive(Clone, Debug, PartialEq)]
struct PushedEvent(u8);

impl AggregateArbitrary for Leaky {
    fn initial_state_strategy() -> BoxedStrategy<Self> {
        Just(Leaky {
            phase: Phase::Open,
            log: Vec::new(),
        })
        .boxed()
    }
    fn command_strategy(_state: &Self) -> BoxedStrategy<Command> {
        Just(Command::Go).boxed()
    }
    fn test_ctx(entropy: Box<dyn EntropySource>, step: u64) -> Self::Ctx {
        OwnedDeterministicCtx {
            entropy,
            actor: 0,
            now: LogicalTime(step),
        }
    }
}

#[test]
#[should_panic(expected = "nondeterminism")]
fn determinism_test_catches_hashmap_iteration_in_decide() {
    run_determinism::<Leaky>(DriveParams {
        cases: 20,
        max_steps: 8,
        seed: 0xBAD,
    });
}
