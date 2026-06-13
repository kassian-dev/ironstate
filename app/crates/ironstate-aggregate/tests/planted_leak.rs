//! Test-the-testers: a deliberately leaky aggregate that `leak_test!`'s driver
//! MUST catch, alongside a clean one it MUST pass.
#![cfg(all(feature = "proptest", feature = "redaction"))]

use ironstate::prelude::*;
use ironstate_aggregate::testkit_support::{DriveParams, run_leak};
use ironstate_aggregate::{
    AggregateArbitrary, AggregateRules, Conceal, EntropySource, LeakTestable, LogicalTime,
    OwnedDeterministicCtx, PerPrincipal, Redact, View,
};
use proptest::prelude::*;

type ParticipantId = u32;

// A hidden secret; others learn nothing (the residue is the unit type).
#[derive(Clone, Debug, PartialEq)]
struct Secret {
    value: u8,
}
impl Conceal for Secret {
    type Concealed = ();
    fn conceal(&self) {}
}

#[derive(StateMachine, Clone, Debug, PartialEq)]
#[state_machine(initial = Live, terminal = [Ended])]
enum Phase {
    Live,
    Ended,
}
#[derive(Event, Clone, Debug, PartialEq)]
enum Step {
    End,
}
impl TransitionRules for Phase {
    type Event = Step;
    fn transition(&self, step: &Step) -> Option<Phase> {
        match (self, step) {
            (Phase::Live, Step::End) => Some(Phase::Ended),
            _ => None,
        }
    }
}

#[derive(Event, Clone, Debug, PartialEq)]
enum Command {
    Sum,
}
#[derive(Clone, Debug, PartialEq)]
enum Ev {
    Summed,
}
#[derive(Debug, thiserror::Error)]
#[error("never")]
struct Never;

// Shared shape: a public total and per-principal hidden secrets.
macro_rules! shared_impls {
    ($ty:ident, $evolve:item) => {
        impl AggregateRules for $ty {
            type Phase = Phase;
            type Command = Command;
            type Event = Ev;
            type Error = Never;
            type Ctx = OwnedDeterministicCtx<ParticipantId>;
            fn phase(&self) -> Phase {
                self.phase.clone()
            }
            fn decide(&self, _cmd: &Command, _ctx: &mut Self::Ctx) -> Result<Vec<Ev>, Never> {
                Ok(vec![Ev::Summed])
            }
            $evolve
        }
        impl AggregateArbitrary for $ty {
            fn initial_state_strategy() -> BoxedStrategy<Self> {
                Just($ty::sample()).boxed()
            }
            fn command_strategy(_state: &Self) -> BoxedStrategy<Command> {
                Just(Command::Sum).boxed()
            }
            fn test_ctx(entropy: Box<dyn EntropySource>, step: u64) -> Self::Ctx {
                OwnedDeterministicCtx { entropy, actor: 0, now: LogicalTime(step) }
            }
        }
        impl LeakTestable for $ty {
            type Principal = ParticipantId;
            fn principals(state: &Self) -> Vec<ParticipantId> {
                state.secrets.iter().map(|(p, _)| *p).collect()
            }
            fn resample_hidden(
                &self,
                principal: &ParticipantId,
                entropy: &mut dyn EntropySource,
            ) -> Self {
                let mut next = self.clone();
                if let Some(secret) = next.secrets.get_mut(principal) {
                    // Resample the hidden value; the residue (unit) is unchanged.
                    secret.value = entropy.draw_range(0..256) as u8;
                }
                next
            }
        }
    };
}

fn sample_secrets() -> PerPrincipal<ParticipantId, Secret> {
    let mut secrets = PerPrincipal::new();
    secrets.insert(1, Secret { value: 5 });
    secrets.insert(2, Secret { value: 7 });
    secrets
}

// --- the leaky aggregate: evolve copies hidden secrets into a public field ---

#[derive(Redact, Clone, Debug)]
#[redact(principal = ParticipantId)]
struct Leaky {
    phase: Phase,
    public_total: u32,
    #[hidden]
    secrets: PerPrincipal<ParticipantId, Secret>,
}
impl Leaky {
    fn sample() -> Self {
        Self {
            phase: Phase::Live,
            public_total: 0,
            secrets: sample_secrets(),
        }
    }
}
shared_impls!(
    Leaky,
    fn evolve(&mut self, event: &Ev) {
        match event {
            // THE PLANTED LEAK: a hidden value flows into a public field.
            Ev::Summed => {
                self.public_total = self.secrets.iter().map(|(_, s)| u32::from(s.value)).sum()
            }
        }
    }
);

// --- the clean aggregate: evolve touches no hidden data -----------------------

#[derive(Redact, Clone, Debug)]
#[redact(principal = ParticipantId)]
struct Clean {
    phase: Phase,
    public_total: u32,
    #[hidden]
    secrets: PerPrincipal<ParticipantId, Secret>,
}
impl Clean {
    fn sample() -> Self {
        Self {
            phase: Phase::Live,
            public_total: 0,
            secrets: sample_secrets(),
        }
    }
}
shared_impls!(
    Clean,
    fn evolve(&mut self, event: &Ev) {
        match event {
            // No covert flow: the public total ignores the hidden secrets.
            Ev::Summed => self.public_total += 1,
        }
    }
);

#[test]
#[should_panic(expected = "leak")]
fn leak_test_catches_hidden_flowing_to_public() {
    run_leak::<Leaky>(
        DriveParams {
            cases: 5,
            max_steps: 4,
            seed: 0x1EA,
        },
        &[],
    );
}

#[test]
fn leak_test_passes_a_clean_aggregate() {
    run_leak::<Clean>(
        DriveParams {
            cases: 5,
            max_steps: 4,
            seed: 0x1EA,
        },
        &[],
    );
}

// Suppress the unused-View-import lint: view_for is used by the leak driver, and
// `View` must be in scope at this crate for the trait bound to resolve.
#[allow(dead_code)]
fn _uses_view(state: &Leaky, p: &ParticipantId) -> <Leaky as View<ParticipantId>>::Output {
    state.view_for(p)
}
