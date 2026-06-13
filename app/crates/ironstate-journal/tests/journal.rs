//! Smoke tests for the journal foundation: the persistent `execute` loop,
//! resume-to-head, replay, the audit digest, fork-position equality, and
//! failed-command atomicity.

use ironstate::prelude::*;
use ironstate_aggregate::{
    Aggregate, AggregateRules, DrawPos, LogicalTime, OwnedDeterministicCtx, Seed, SeededEntropy,
    StableHash, audit_digest,
};
use ironstate_journal::{
    ExecuteError, Journal, MemoryJournal, Seq, Snapshot, execute, replay_hash, resume,
};

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

fn genesis() -> Counter {
    Counter {
        phase: Phase::Open,
        total: 0,
    }
}

/// Build a context whose entropy is positioned at the journal's head — exactly
/// what `execute` expects of a live stream.
fn ctx_at_head(journal: &MemoryJournal<Counter>, seed: &Seed) -> OwnedDeterministicCtx<u32> {
    let pos = journal
        .head()
        .map_or(DrawPos(0), |h| journal.entropy_pos(h).unwrap());
    OwnedDeterministicCtx {
        entropy: Box::new(SeededEntropy::at(seed, pos)),
        actor: 0,
        now: LogicalTime(0),
    }
}

#[test]
fn execute_resume_and_replay_agree() {
    let seed = Seed([9; 32]);
    let mut journal = MemoryJournal::new(genesis());
    let mut agg = Aggregate::new(genesis()).unwrap();

    for _ in 0..4 {
        let mut ctx = ctx_at_head(&journal, &seed);
        execute(&mut journal, &mut agg, &Command::Tick, &mut ctx).unwrap();
    }

    // resume rebuilds the exact same state from the journal.
    let (resumed, _entropy) = resume::<Counter, _>(&journal, &seed).unwrap();
    assert_eq!(resumed.state(), agg.state());

    // replay_hash over the genesis snapshot + all events matches the live digest.
    let snapshot = Snapshot {
        state: genesis(),
        schema_version: 0,
        at: Seq(0),
        entropy_pos: DrawPos(0),
    };
    let events = journal.events_since(None).unwrap();
    assert_eq!(
        replay_hash(snapshot, &events).unwrap(),
        audit_digest(agg.state())
    );
}

#[test]
fn fork_agrees_on_position_at_the_fork_point() {
    let seed = Seed([3; 32]);
    let mut journal = MemoryJournal::new(genesis());
    let mut agg = Aggregate::new(genesis()).unwrap();
    for _ in 0..3 {
        let mut ctx = ctx_at_head(&journal, &seed);
        execute(&mut journal, &mut agg, &Command::Tick, &mut ctx).unwrap();
    }

    let forked = journal.fork(Seq(2)).unwrap();
    assert_eq!(
        forked.entropy_pos(Seq(2)).unwrap(),
        journal.entropy_pos(Seq(2)).unwrap(),
    );
    // The fork is independent: appends to one do not change the other.
    assert_eq!(forked.head(), Some(Seq(2)));
    assert_eq!(journal.head(), Some(Seq(3)));
}

#[test]
fn a_rejected_command_changes_nothing() {
    let seed = Seed([1; 32]);
    let mut journal = MemoryJournal::new(genesis());
    let mut agg = Aggregate::new(genesis()).unwrap();

    let mut ctx = ctx_at_head(&journal, &seed);
    execute(&mut journal, &mut agg, &Command::Close, &mut ctx).unwrap();
    let head_before = journal.head();

    // The phase is now terminal; a further command is rejected and leaves the
    // journal head and the position untouched.
    let mut ctx = ctx_at_head(&journal, &seed);
    let position_before = ctx.entropy.draws();
    let err = execute(&mut journal, &mut agg, &Command::Tick, &mut ctx).unwrap_err();
    assert!(matches!(err, ExecuteError::Rejected(_)));
    assert_eq!(journal.head(), head_before);
    assert_eq!(ctx.entropy.draws(), position_before);
}
