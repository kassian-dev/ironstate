//! The aggregate runtime: the decide/evolve law, structural enforcement via the
//! phase machine, init checks, and probe-backed `why_not`.

use ironstate::prelude::*;
use ironstate_aggregate::{
    Aggregate, AggregateRules, InitError, LogicalTime, OwnedDeterministicCtx, Rejection, Seed,
    SeededEntropy,
};

// --- the phase machine (a real core state machine) ------------------------

#[derive(StateMachine, Clone, Debug, PartialEq)]
#[state_machine(initial = Setup, terminal = [Done])]
enum GamePhase {
    Setup,
    #[only_accepts(kind = "play")]
    Playing,
    Done,
}

#[derive(Event, Clone, Debug, PartialEq)]
enum PhaseStep {
    Begin,
    End,
}

impl TransitionRules for GamePhase {
    type Event = PhaseStep;
    fn transition(&self, step: &PhaseStep) -> Option<GamePhase> {
        use GamePhase::*;
        use PhaseStep::*;
        match (self, step) {
            (Setup, Begin) => Some(Playing),
            (Playing, End) => Some(Done),
            _ => None,
        }
    }
}

// --- the aggregate --------------------------------------------------------

#[derive(Event, Clone, Debug, PartialEq)]
enum Command {
    Begin,
    #[event_kind = "play"]
    Roll,
    #[event_kind = "play"]
    Finish,
}

#[derive(Clone, Debug, PartialEq)]
enum GameEvent {
    Started,
    Rolled(u8),
    Finished,
}

#[derive(Debug, thiserror::Error)]
enum GameError {
    #[error("the game is not in progress")]
    NotPlaying,
    #[error("the game has not started")]
    NotStarted,
}

#[derive(Clone, Debug, PartialEq)]
struct Game {
    phase: GamePhase,
    rolls: Vec<u8>,
    total: u32,
}

impl Game {
    fn new() -> Self {
        Self {
            phase: GamePhase::Setup,
            rolls: Vec::new(),
            total: 0,
        }
    }
}

impl AggregateRules for Game {
    type Phase = GamePhase;
    type Command = Command;
    type Event = GameEvent;
    type Error = GameError;
    type Ctx = OwnedDeterministicCtx<u32>;

    fn phase(&self) -> GamePhase {
        self.phase.clone()
    }

    fn decide(&self, cmd: &Command, ctx: &mut Self::Ctx) -> Result<Vec<GameEvent>, GameError> {
        match cmd {
            Command::Begin => {
                if self.phase != GamePhase::Setup {
                    return Err(GameError::NotStarted);
                }
                Ok(vec![GameEvent::Started])
            }
            Command::Roll => {
                if self.phase != GamePhase::Playing {
                    return Err(GameError::NotPlaying);
                }
                // The one place entropy is drawn: a 1..=6 die roll.
                let die = ctx.entropy.draw_range(1..7) as u8;
                Ok(vec![GameEvent::Rolled(die)])
            }
            Command::Finish => {
                if self.phase != GamePhase::Playing {
                    return Err(GameError::NotPlaying);
                }
                Ok(vec![GameEvent::Finished])
            }
        }
    }

    fn evolve(&mut self, event: &GameEvent) {
        match event {
            GameEvent::Started => self.phase = GamePhase::Playing,
            GameEvent::Rolled(n) => {
                self.rolls.push(*n);
                self.total += u32::from(*n);
            }
            GameEvent::Finished => self.phase = GamePhase::Done,
        }
    }
}

fn ctx_at(pos: u64) -> OwnedDeterministicCtx<u32> {
    OwnedDeterministicCtx {
        entropy: Box::new(SeededEntropy::at(
            &Seed([42u8; 32]),
            ironstate_aggregate::DrawPos(pos),
        )),
        actor: 1,
        now: LogicalTime(0),
    }
}

#[test]
fn new_rejects_a_non_initial_phase() {
    let started = Game {
        phase: GamePhase::Playing,
        rolls: vec![],
        total: 0,
    };
    let err = Aggregate::new(started).unwrap_err();
    assert!(matches!(
        err,
        InitError::NotInitialPhase {
            found: GamePhase::Playing
        }
    ));

    // A fresh game in the initial phase is accepted.
    assert!(Aggregate::new(Game::new()).is_ok());
}

#[test]
fn handle_equals_decide_then_evolve() {
    // Run a command through handle...
    let mut agg = Aggregate::new(Game::new()).unwrap();
    agg.handle(&Command::Begin, &mut ctx_at(0)).unwrap();
    let mut ctx = ctx_at(0);
    let via_handle = agg.handle(&Command::Roll, &mut ctx).unwrap();

    // ...and the same command as decide-then-evolve from the same state and the
    // same entropy position. The events and resulting state must match.
    let mut manual = Game {
        phase: GamePhase::Playing,
        rolls: vec![],
        total: 0,
    };
    let mut ctx2 = ctx_at(0);
    let via_manual = manual.decide(&Command::Roll, &mut ctx2).unwrap();
    for event in &via_manual {
        manual.evolve(event);
    }

    assert_eq!(via_handle, via_manual);
    assert_eq!(agg.state(), &manual);
}

#[test]
fn terminal_phase_rejects_commands() {
    let mut agg = Aggregate::new(Game::new()).unwrap();
    agg.handle(&Command::Begin, &mut ctx_at(0)).unwrap();
    agg.handle(&Command::Finish, &mut ctx_at(0)).unwrap();
    assert_eq!(agg.phase(), GamePhase::Done);

    let rejection = agg.handle(&Command::Roll, &mut ctx_at(0)).unwrap_err();
    assert!(matches!(rejection, Rejection::TerminalPhase { .. }));
}

#[test]
fn phase_gates_commands_by_kind() {
    let mut agg = Aggregate::new(Game::new()).unwrap();
    agg.handle(&Command::Begin, &mut ctx_at(0)).unwrap();
    assert_eq!(agg.phase(), GamePhase::Playing);

    // Playing only accepts kind "play"; Begin carries the default kind.
    let rejection = agg.handle(&Command::Begin, &mut ctx_at(0)).unwrap_err();
    match rejection {
        Rejection::CommandKindRejected { expected_kinds, .. } => {
            assert_eq!(expected_kinds, &[Kind("play")]);
        }
        other => panic!("expected CommandKindRejected, got {other:?}"),
    }
}

#[test]
fn domain_rejection_mutates_nothing() {
    let mut agg = Aggregate::new(Game::new()).unwrap();
    // Roll in Setup is a domain rejection (NotPlaying); no event, no mutation.
    let before = agg.state().clone();
    let rejection = agg.handle(&Command::Roll, &mut ctx_at(0)).unwrap_err();
    assert!(matches!(
        rejection,
        Rejection::Domain(GameError::NotPlaying)
    ));
    assert_eq!(agg.state(), &before);
}

#[test]
fn why_not_with_a_probe_does_not_advance_the_stream() {
    let mut agg = Aggregate::new(Game::new()).unwrap();
    agg.handle(&Command::Begin, &mut ctx_at(0)).unwrap();

    let ctx = ctx_at(0);
    let before = ctx.entropy.draws();

    // why_not runs decide (which would draw) against a probe, so the journaled
    // stream is untouched.
    let mut probing = OwnedDeterministicCtx {
        entropy: ctx.entropy.probe(),
        actor: ctx.actor,
        now: ctx.now,
    };
    assert!(agg.why_not(&Command::Roll, &mut probing).is_none());
    assert_eq!(ctx.entropy.draws(), before);
}
