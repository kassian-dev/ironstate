#![doc = include_str!("../README.md")]

use anyhow::Result;
use ironstate::prelude::*;
use ironstate_aggregate::{
    Aggregate, AggregateArbitrary, AggregateInvariant, AggregateInvariants, AggregateRules,
    Conceal, DrawPos, EntropySource, LeakTestable, LogicalTime, Owned, OwnedDeterministicCtx,
    PerPrincipal, Redact, Seed, SeededEntropy, StableHash, View,
};
use ironstate_journal::{
    Journal, MemoryJournal, React, Seq, StreamId, Subscription, execute, resume,
};
use proptest::prelude::*;

type ParticipantId = u32;

// --- the phase machine ----------------------------------------------------

#[derive(StateMachine, StableHash, Clone, Debug, PartialEq)]
#[state_machine(initial = Setup, terminal = [Resolved])]
enum MatchPhase {
    Setup,
    // During play only game moves and system (timeout) commands are accepted.
    #[only_accepts(kind = ["play", "system"])]
    Playing,
    Resolved,
}

#[derive(Event, Clone, Debug, PartialEq)]
enum PhaseStep {
    Start,
    Finish,
}

impl TransitionRules for MatchPhase {
    type Event = PhaseStep;
    fn transition(&self, step: &PhaseStep) -> Option<MatchPhase> {
        use MatchPhase::*;
        use PhaseStep::*;
        match (self, step) {
            (Setup, Start) => Some(Playing),
            (Playing, Finish) => Some(Resolved),
            _ => None,
        }
    }
}

// --- hidden value types ---------------------------------------------------

#[derive(StableHash, Clone, Debug, PartialEq)]
struct Hand {
    cards: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq)]
struct HandPublic {
    count: u8,
}
impl Conceal for Hand {
    type Concealed = HandPublic;
    fn conceal(&self) -> HandPublic {
        HandPublic {
            count: self.cards.len() as u8,
        }
    }
}

#[derive(StableHash, Clone, Debug, PartialEq)]
struct Deck {
    cards: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq)]
struct DeckPublic {
    count: u8,
}
impl Conceal for Deck {
    type Concealed = DeckPublic;
    fn conceal(&self) -> DeckPublic {
        DeckPublic {
            count: self.cards.len() as u8,
        }
    }
}

#[derive(StableHash, Clone, Debug, PartialEq)]
struct Secret {
    token: u32,
}
impl Conceal for Secret {
    // Non-owners learn nothing at all.
    type Concealed = ();
    fn conceal(&self) {}
}

// --- the aggregate --------------------------------------------------------

#[derive(Event, Clone, Debug, PartialEq)]
enum Command {
    // Setup: the default kind.
    Join {
        who: ParticipantId,
    },
    #[event_kind = "play"]
    Draw {
        who: ParticipantId,
    },
    #[event_kind = "play"]
    PlayCard {
        who: ParticipantId,
    },
    #[event_kind = "play"]
    EndTurn {
        who: ParticipantId,
    },
    // Minted by the embedding layer when continuous absence is detected.
    #[event_kind = "system"]
    Timeout,
}

#[derive(Clone, Debug, PartialEq)]
enum MatchEvent {
    Joined(ParticipantId),
    Started,
    Drew { who: ParticipantId, card: u8 },
    Played { who: ParticipantId, card: u8 },
    TurnEnded,
    Resolved,
}

#[derive(Debug, thiserror::Error)]
enum MatchError {
    #[error("the match is not accepting players")]
    NotSetup,
    #[error("the match is not in progress")]
    NotPlaying,
    #[error("it is not that participant's turn")]
    NotYourTurn,
    #[error("that participant has already joined")]
    AlreadyJoined,
    #[error("the hand is empty")]
    EmptyHand,
}

#[derive(Redact, StableHash, Clone, Debug, PartialEq)]
#[redact(principal = ParticipantId)]
struct MatchState {
    phase: MatchPhase,
    board: Vec<u8>,
    participants: Vec<ParticipantId>,
    turn: u32,
    #[hidden]
    hands: PerPrincipal<ParticipantId, Hand>,
    #[hidden]
    fabrication: Owned<ParticipantId, Secret>,
    #[hidden(conceal)]
    decks: PerPrincipal<ParticipantId, Deck>,
}

impl MatchState {
    fn new() -> Self {
        Self {
            phase: MatchPhase::Setup,
            board: Vec::new(),
            participants: Vec::new(),
            turn: 0,
            hands: PerPrincipal::new(),
            fabrication: Owned::new(0, Secret { token: 0 }),
            decks: PerPrincipal::new(),
        }
    }

    fn active(&self) -> Option<ParticipantId> {
        self.participants
            .get(self.turn as usize % self.participants.len().max(1))
            .copied()
    }
}

impl AggregateRules for MatchState {
    type Phase = MatchPhase;
    type Command = Command;
    type Event = MatchEvent;
    type Error = MatchError;
    type Ctx = OwnedDeterministicCtx<ParticipantId>;

    fn phase(&self) -> MatchPhase {
        self.phase.clone()
    }

    fn decide(&self, cmd: &Command, ctx: &mut Self::Ctx) -> Result<Vec<MatchEvent>, MatchError> {
        match cmd {
            Command::Join { who } => {
                if self.phase != MatchPhase::Setup {
                    return Err(MatchError::NotSetup);
                }
                if self.participants.contains(who) {
                    return Err(MatchError::AlreadyJoined);
                }
                let mut events = vec![MatchEvent::Joined(*who)];
                // The match starts once the second player joins.
                if self.participants.len() + 1 >= 2 {
                    events.push(MatchEvent::Started);
                }
                Ok(events)
            }
            Command::Draw { who } => {
                self.require_turn(*who)?;
                // The one place entropy is drawn: which card is drawn.
                let card = ctx.entropy.draw_range(0..52) as u8;
                Ok(vec![MatchEvent::Drew { who: *who, card }])
            }
            Command::PlayCard { who } => {
                self.require_turn(*who)?;
                let card = self
                    .hands
                    .get(who)
                    .and_then(|h| h.cards.first().copied())
                    .ok_or(MatchError::EmptyHand)?;
                Ok(vec![MatchEvent::Played { who: *who, card }])
            }
            Command::EndTurn { who } => {
                self.require_turn(*who)?;
                Ok(vec![MatchEvent::TurnEnded])
            }
            Command::Timeout => {
                if self.phase != MatchPhase::Playing {
                    return Err(MatchError::NotPlaying);
                }
                Ok(vec![MatchEvent::Resolved])
            }
        }
    }

    fn evolve(&mut self, event: &MatchEvent) {
        match event {
            MatchEvent::Joined(p) => {
                self.participants.push(*p);
                self.hands.insert(*p, Hand { cards: Vec::new() });
                self.decks.insert(
                    *p,
                    Deck {
                        cards: (0..5).collect(),
                    },
                );
            }
            MatchEvent::Started => self.phase = MatchPhase::Playing,
            MatchEvent::Drew { who, card } => {
                if let Some(hand) = self.hands.get_mut(who) {
                    hand.cards.push(*card);
                }
            }
            MatchEvent::Played { who, card } => {
                if let Some(hand) = self.hands.get_mut(who)
                    && let Some(pos) = hand.cards.iter().position(|c| c == card)
                {
                    hand.cards.remove(pos);
                }
                self.board.push(*card);
            }
            MatchEvent::TurnEnded => {
                let len = self.participants.len().max(1) as u32;
                self.turn = (self.turn + 1) % len;
            }
            MatchEvent::Resolved => self.phase = MatchPhase::Resolved,
        }
    }
}

impl MatchState {
    fn require_turn(&self, who: ParticipantId) -> Result<(), MatchError> {
        if self.phase != MatchPhase::Playing {
            return Err(MatchError::NotPlaying);
        }
        if self.active() != Some(who) {
            return Err(MatchError::NotYourTurn);
        }
        Ok(())
    }
}

impl AggregateInvariants for MatchState {
    fn invariants() -> Vec<AggregateInvariant<Self>> {
        vec![
            AggregateInvariant::<Self>::custom("the board only grows")
                .assert(|before, _event, after| after.board.len() >= before.board.len()),
        ]
    }
}

impl AggregateArbitrary for MatchState {
    fn initial_state_strategy() -> BoxedStrategy<Self> {
        Just(MatchState::new()).boxed()
    }

    fn command_strategy(state: &Self) -> BoxedStrategy<Command> {
        match state.phase {
            MatchPhase::Setup => {
                // The next player to join takes the next id.
                let who = state.participants.len() as ParticipantId;
                Just(Command::Join { who }).boxed()
            }
            MatchPhase::Playing => {
                let who = state.active().unwrap_or(0);
                prop_oneof![
                    5 => Just(Command::Draw { who }),
                    3 => Just(Command::PlayCard { who }),
                    3 => Just(Command::EndTurn { who }),
                    1 => Just(Command::Timeout),
                ]
                .boxed()
            }
            // Terminal: never sampled, but the strategy must exist.
            MatchPhase::Resolved => Just(Command::Timeout).boxed(),
        }
    }

    fn test_ctx(entropy: Box<dyn EntropySource>, step: u64) -> Self::Ctx {
        OwnedDeterministicCtx {
            entropy,
            actor: 0,
            now: LogicalTime(step),
        }
    }
}

impl LeakTestable for MatchState {
    type Principal = ParticipantId;

    fn principals(state: &Self) -> Vec<ParticipantId> {
        state.participants.clone()
    }

    fn resample_hidden(&self, principal: &ParticipantId, entropy: &mut dyn EntropySource) -> Self {
        let mut next = self.clone();
        // Resample the principal's hand, keeping its size (its public residue).
        if let Some(hand) = next.hands.get_mut(principal) {
            for card in &mut hand.cards {
                *card = entropy.draw_range(0..52) as u8;
            }
        }
        // Resample the fabrication if this principal owns it (residue is unit).
        if next.fabrication.owner() == principal {
            next.fabrication = Owned::new(
                *principal,
                Secret {
                    token: entropy.draw_u64() as u32,
                },
            );
        }
        next
    }
}

// --- a second aggregate the match feeds via a subscription ----------------

#[derive(StateMachine, Clone, Debug, PartialEq)]
#[state_machine(initial = Active, terminal = [Closed])]
enum ProfilePhase {
    Active,
    Closed,
}
#[derive(Event, Clone, Debug, PartialEq)]
enum ProfileStep {
    Close,
}
impl TransitionRules for ProfilePhase {
    type Event = ProfileStep;
    fn transition(&self, _: &ProfileStep) -> Option<ProfilePhase> {
        matches!(self, ProfilePhase::Active).then_some(ProfilePhase::Closed)
    }
}
#[derive(Event, Clone, Debug, PartialEq)]
enum ProfileCommand {
    RecordResolved,
}
#[derive(Clone, Debug, PartialEq)]
enum ProfileEvent {
    Recorded,
}
#[derive(Debug, thiserror::Error)]
#[error("never")]
struct ProfileError;

#[derive(Clone, Debug, PartialEq)]
struct PlayerProfile {
    phase: ProfilePhase,
    matches_resolved: u32,
}
impl AggregateRules for PlayerProfile {
    type Phase = ProfilePhase;
    type Command = ProfileCommand;
    type Event = ProfileEvent;
    type Error = ProfileError;
    type Ctx = OwnedDeterministicCtx<ParticipantId>;
    fn phase(&self) -> ProfilePhase {
        self.phase.clone()
    }
    fn decide(
        &self,
        _: &ProfileCommand,
        _: &mut Self::Ctx,
    ) -> Result<Vec<ProfileEvent>, ProfileError> {
        Ok(vec![ProfileEvent::Recorded])
    }
    fn evolve(&mut self, _: &ProfileEvent) {
        self.matches_resolved += 1;
    }
}

// A profile records each resolved match it sees on the source stream.
impl React<MatchState> for PlayerProfile {
    fn react(&self, event: &MatchEvent, _at: Seq) -> Vec<ProfileCommand> {
        match event {
            MatchEvent::Resolved => vec![ProfileCommand::RecordResolved],
            _ => Vec::new(),
        }
    }
}

// --- the demo -------------------------------------------------------------

fn ctx(seed: &Seed, pos: DrawPos, actor: ParticipantId) -> OwnedDeterministicCtx<ParticipantId> {
    OwnedDeterministicCtx {
        entropy: Box::new(SeededEntropy::at(seed, pos)),
        actor,
        now: LogicalTime(0),
    }
}

fn run_demo() -> Result<()> {
    let seed = Seed([0x5A; 32]);
    let mut journal = MemoryJournal::new(MatchState::new());
    let mut game = Aggregate::new(MatchState::new())?;

    // Two players join (the second triggers Started), then player 0 draws twice.
    let script = [
        Command::Join { who: 0 },
        Command::Join { who: 1 },
        Command::Draw { who: 0 },
        Command::Draw { who: 0 },
    ];
    for command in &script {
        let pos = journal
            .head()
            .map_or(DrawPos(0), |h| journal.entropy_pos(h).unwrap());
        let mut context = ctx(&seed, pos, 0);
        // Ignore rejections in the demo; a real server would surface them.
        let _ = execute(&mut journal, &mut game, command, &mut context);
    }

    // Redaction: each participant sees their own hand, only counts of others'.
    let view0 = game.view_for(&0);
    let view1 = game.view_for(&1);
    println!("player 0 sees their hand: {:?}", view0.hands.mine);
    println!(
        "player 0 sees player 1 only as: {:?}",
        view0.hands.others.get(&1)
    );
    println!("player 1 sees their hand: {:?}", view1.hands.mine);
    assert_eq!(view0.board, view1.board, "the board is public to everyone");

    // Resume rebuilds the same state from the journal.
    let (resumed, _entropy) = resume::<MatchState, _>(&journal, &seed)
        .map_err(|e| anyhow::anyhow!("resume failed: {e}"))?;
    assert_eq!(resumed.state(), game.state());
    println!("resumed state matches the live state");

    // A system-minted timeout resolves the match.
    let pos = journal
        .head()
        .map_or(DrawPos(0), |h| journal.entropy_pos(h).unwrap());
    let mut context = ctx(&seed, pos, 0);
    execute(&mut journal, &mut game, &Command::Timeout, &mut context)
        .map_err(|e| anyhow::anyhow!("timeout failed: {e}"))?;
    println!("match phase after timeout: {:?}", game.phase());

    // Feed the match's events to a player profile via a subscription. The
    // Resolved event records one match; redelivering it is idempotent.
    let mut profile_journal = MemoryJournal::new(PlayerProfile {
        phase: ProfilePhase::Active,
        matches_resolved: 0,
    });
    let mut profile = Aggregate::new(PlayerProfile {
        phase: ProfilePhase::Active,
        matches_resolved: 0,
    })?;
    let mut subscription: Subscription<MatchState, PlayerProfile> = Subscription::new();
    let stream = StreamId::new("match-1");
    let mut profile_ctx = ctx(&seed, DrawPos(0), 0);
    for (i, event) in journal.events_since(None).unwrap().iter().enumerate() {
        subscription
            .deliver(
                &stream,
                Seq(i as u64 + 1),
                &event.event,
                &mut profile,
                &mut profile_ctx,
                &mut profile_journal,
            )
            .map_err(|e| anyhow::anyhow!("delivery failed: {e}"))?;
    }
    println!(
        "profile recorded {} resolved match(es)",
        profile.state().matches_resolved
    );

    Ok(())
}

fn main() -> Result<()> {
    run_demo()
}

#[cfg(test)]
mod tests {
    use super::*;

    // All five family test macros on the worked aggregate.
    ironstate_aggregate::test!(MatchState, cases = 200, max_steps = 40);
    ironstate_aggregate::determinism_test!(MatchState, cases = 100, max_steps = 40);
    // PlayCard legitimately moves a hidden card to the public board, so it is
    // excluded from the covert-leak property.
    ironstate_aggregate::leak_test!(
        MatchState,
        cases = 200,
        max_steps = 30,
        excluding = [PlayCard]
    );
    ironstate_journal::journal_contract_test!(MatchState);
    ironstate_journal::scenario_test!(MatchState, cases = 150, max_steps = 40);

    #[test]
    fn the_demo_runs() {
        run_demo().unwrap();
    }
}
