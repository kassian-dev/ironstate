//! A subscription delivers a source stream to a target exactly once: duplicates
//! and out-of-order (older) redeliveries are dropped, so the target converges to
//! the same state as exactly-once delivery.

use ironstate::prelude::*;
use ironstate_aggregate::{
    Aggregate, AggregateRules, LogicalTime, OwnedDeterministicCtx, Seed, SeededEntropy,
};
use ironstate_journal::{Delivered, MemoryJournal, React, Seq, StreamId, Subscription};

// --- the source aggregate (only its event type is used here) --------------

#[derive(StateMachine, Clone, Debug, PartialEq)]
#[state_machine(initial = Live, terminal = [Done])]
enum SrcPhase {
    Live,
    Done,
}
#[derive(Event, Clone, Debug, PartialEq)]
enum SrcStep {
    End,
}
impl TransitionRules for SrcPhase {
    type Event = SrcStep;
    fn transition(&self, _: &SrcStep) -> Option<SrcPhase> {
        matches!(self, SrcPhase::Live).then_some(SrcPhase::Done)
    }
}
#[derive(Event, Clone, Debug, PartialEq)]
enum SrcCommand {
    Ping,
}
#[derive(Clone, Debug, PartialEq)]
enum SrcEvent {
    Pinged,
}
#[derive(Debug, thiserror::Error)]
#[error("never")]
struct SrcErr;

#[derive(Clone, Debug, PartialEq)]
struct Source {
    phase: SrcPhase,
}
impl AggregateRules for Source {
    type Phase = SrcPhase;
    type Command = SrcCommand;
    type Event = SrcEvent;
    type Error = SrcErr;
    type Ctx = OwnedDeterministicCtx<u32>;
    fn phase(&self) -> SrcPhase {
        self.phase.clone()
    }
    fn decide(&self, _: &SrcCommand, _: &mut Self::Ctx) -> Result<Vec<SrcEvent>, SrcErr> {
        Ok(vec![SrcEvent::Pinged])
    }
    fn evolve(&mut self, _: &SrcEvent) {}
}

// --- the target aggregate, which tallies source pings ----------------------

#[derive(StateMachine, Clone, Debug, PartialEq)]
#[state_machine(initial = Open, terminal = [Shut])]
enum TallyPhase {
    Open,
    Shut,
}
#[derive(Event, Clone, Debug, PartialEq)]
enum TallyStep {
    Shut,
}
impl TransitionRules for TallyPhase {
    type Event = TallyStep;
    fn transition(&self, _: &TallyStep) -> Option<TallyPhase> {
        matches!(self, TallyPhase::Open).then_some(TallyPhase::Shut)
    }
}
#[derive(Event, Clone, Debug, PartialEq)]
enum TallyCommand {
    Bump,
}
#[derive(Clone, Debug, PartialEq)]
enum TallyEvent {
    Bumped,
}
#[derive(Debug, thiserror::Error)]
#[error("never")]
struct TallyErr;

#[derive(Clone, Debug, PartialEq)]
struct Tally {
    phase: TallyPhase,
    count: u32,
}
impl AggregateRules for Tally {
    type Phase = TallyPhase;
    type Command = TallyCommand;
    type Event = TallyEvent;
    type Error = TallyErr;
    type Ctx = OwnedDeterministicCtx<u32>;
    fn phase(&self) -> TallyPhase {
        self.phase.clone()
    }
    fn decide(&self, _: &TallyCommand, _: &mut Self::Ctx) -> Result<Vec<TallyEvent>, TallyErr> {
        Ok(vec![TallyEvent::Bumped])
    }
    fn evolve(&mut self, _: &TallyEvent) {
        self.count += 1;
    }
}

impl React<Source> for Tally {
    fn react(&self, _event: &SrcEvent, _at: Seq) -> Vec<TallyCommand> {
        vec![TallyCommand::Bump]
    }
}

fn ctx() -> OwnedDeterministicCtx<u32> {
    OwnedDeterministicCtx {
        entropy: Box::new(SeededEntropy::from_seed(&Seed([0; 32]))),
        actor: 0,
        now: LogicalTime(0),
    }
}

#[test]
fn duplicates_and_reorders_converge_to_exactly_once() {
    let mut journal = MemoryJournal::new(Tally {
        phase: TallyPhase::Open,
        count: 0,
    });
    let mut target = Aggregate::new(Tally {
        phase: TallyPhase::Open,
        count: 0,
    })
    .unwrap();
    let mut subscription: Subscription<Source, Tally> = Subscription::new();
    let stream = StreamId::new("match-1");
    let mut context = ctx();

    let deliver = |sub: &mut Subscription<Source, Tally>,
                   journal: &mut MemoryJournal<Tally>,
                   target: &mut Aggregate<Tally>,
                   context: &mut OwnedDeterministicCtx<u32>,
                   at: u64| {
        sub.deliver(
            &stream,
            Seq(at),
            &SrcEvent::Pinged,
            target,
            context,
            journal,
        )
        .unwrap()
    };

    assert_eq!(
        deliver(
            &mut subscription,
            &mut journal,
            &mut target,
            &mut context,
            1
        ),
        Delivered::Applied
    );
    assert_eq!(
        deliver(
            &mut subscription,
            &mut journal,
            &mut target,
            &mut context,
            2
        ),
        Delivered::Applied
    );
    // Duplicate of Seq 2 is dropped.
    assert_eq!(
        deliver(
            &mut subscription,
            &mut journal,
            &mut target,
            &mut context,
            2
        ),
        Delivered::Duplicate
    );
    // Older, out-of-order Seq 1 is dropped.
    assert_eq!(
        deliver(
            &mut subscription,
            &mut journal,
            &mut target,
            &mut context,
            1
        ),
        Delivered::Duplicate
    );
    assert_eq!(
        deliver(
            &mut subscription,
            &mut journal,
            &mut target,
            &mut context,
            3
        ),
        Delivered::Applied
    );

    // Three distinct, increasing seqs applied — exactly-once delivery's result.
    assert_eq!(target.state().count, 3);
    assert_eq!(subscription.mark(&stream), Some(Seq(3)));
}
