//! The aggregate trait and runtime: decide/evolve with structural enforcement.

use ironstate::{EventKind, Kind, StateMachine};

/// A consistency boundary whose state is a struct, changed by applying facts.
///
/// Two functions, two laws. `decide` is the only place rules live and the only
/// function permitted to draw entropy; it validates intent and emits the facts
/// that follow, mutating nothing. `evolve` is total, infallible, and pure — it
/// applies one fact mechanically, drawing no entropy and reading no clock. So
/// replay (a sequence of `evolve`s) consumes no entropy, which is why the
/// journal must record positions rather than recompute them.
pub trait AggregateRules: Sized {
    /// The phase machine — a real core state machine whose structure (initial,
    /// terminal, command-kind restrictions) the runtime reuses.
    type Phase: StateMachine;
    /// Intent that may be rejected. Carries `#[event_kind]` like a core event,
    /// so a phase's `#[only_accepts]` can gate it.
    type Command: EventKind + core::fmt::Debug;
    /// A fact that has happened and can only be applied.
    type Event: core::fmt::Debug + Clone;
    /// Domain rejections from `decide`.
    type Error: std::error::Error;
    /// The decision context (entropy, actor, logical time).
    type Ctx;

    /// The current phase.
    fn phase(&self) -> Self::Phase;

    /// Validate `cmd` against the current state and emit the events that follow.
    /// The only place entropy may be drawn. Does not mutate state.
    fn decide(
        &self,
        cmd: &Self::Command,
        ctx: &mut Self::Ctx,
    ) -> Result<Vec<Self::Event>, Self::Error>;

    /// Apply one fact. Total, infallible, pure: never panics, draws no entropy,
    /// reads no clock.
    fn evolve(&mut self, event: &Self::Event);
}

/// Why a command was rejected.
///
/// `Display` is teaching prose; `Domain` forwards its inner error transparently.
#[non_exhaustive]
pub enum Rejection<A: AggregateRules> {
    /// The current phase is terminal and accepts no commands.
    TerminalPhase {
        /// The terminal phase.
        phase: A::Phase,
    },
    /// The command's kind is not accepted by the current phase.
    CommandKindRejected {
        /// The phase that rejected the command.
        phase: A::Phase,
        /// The kinds this phase accepts.
        expected_kinds: &'static [Kind],
    },
    /// A domain rejection from `decide`.
    Domain(A::Error),
}

impl<A: AggregateRules> core::fmt::Debug for Rejection<A> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TerminalPhase { phase } => f
                .debug_struct("TerminalPhase")
                .field("phase", phase)
                .finish(),
            Self::CommandKindRejected {
                phase,
                expected_kinds,
            } => f
                .debug_struct("CommandKindRejected")
                .field("phase", phase)
                .field("expected_kinds", expected_kinds)
                .finish(),
            Self::Domain(error) => f.debug_tuple("Domain").field(error).finish(),
        }
    }
}

impl<A: AggregateRules> core::fmt::Display for Rejection<A> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TerminalPhase { phase } => write!(
                f,
                "command rejected because phase {phase:?} is terminal.\n\
                 A terminal phase accepts no further commands.\n\
                 The aggregate's life is over; start a new one if the domain allows it.",
            ),
            Self::CommandKindRejected {
                phase,
                expected_kinds,
            } => write!(
                f,
                "command rejected by phase {phase:?} on an event-kind mismatch.\n\
                 {phase:?} only accepts commands of kind {expected_kinds:?}.\n\
                 Annotate the command with a matching `#[event_kind = …]`, or relax the \
                 phase's `#[only_accepts(kind = …)]`.",
            ),
            Self::Domain(error) => write!(f, "{error}"),
        }
    }
}

impl<A: AggregateRules> std::error::Error for Rejection<A> {}

/// Why constructing an [`Aggregate`] failed.
#[non_exhaustive]
pub enum InitError<A: AggregateRules> {
    /// `new` was given a state whose phase is not the phase machine's initial.
    NotInitialPhase {
        /// The non-initial phase the state was in.
        found: A::Phase,
    },
}

impl<A: AggregateRules> core::fmt::Debug for InitError<A> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotInitialPhase { found } => f
                .debug_struct("NotInitialPhase")
                .field("found", found)
                .finish(),
        }
    }
}

impl<A: AggregateRules> core::fmt::Display for InitError<A> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotInitialPhase { found } => write!(
                f,
                "cannot start an aggregate in phase {found:?}.\n\
                 `new` builds a fresh aggregate, which must be in the phase machine's \
                 initial phase.\n\
                 To load a past state instead, replay it from the journal rather than \
                 constructing it directly.",
            ),
        }
    }
}

impl<A: AggregateRules> std::error::Error for InitError<A> {}

/// A running aggregate: the state plus the structural enforcement its phase
/// machine defines.
pub struct Aggregate<A: AggregateRules> {
    state: A,
}

impl<A: AggregateRules + core::fmt::Debug> core::fmt::Debug for Aggregate<A> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Aggregate")
            .field("state", &self.state)
            .finish()
    }
}

impl<A: AggregateRules> Aggregate<A> {
    /// Build a fresh aggregate. The state must be in the phase machine's initial
    /// phase.
    pub fn new(state: A) -> Result<Self, InitError<A>> {
        let phase = state.phase();
        if phase == <A::Phase as StateMachine>::initial() {
            Ok(Self { state })
        } else {
            Err(InitError::NotInitialPhase { found: phase })
        }
    }

    /// Construct an aggregate at an arbitrary (past-initial) state, skipping the
    /// initial-phase check.
    ///
    /// This is the replay constructor: a snapshot or a replayed event stream is
    /// past the initial phase by right. Prefer `new` for fresh aggregates.
    pub fn from_state(state: A) -> Self {
        Self { state }
    }

    /// The current state.
    pub fn state(&self) -> &A {
        &self.state
    }

    /// The current phase.
    pub fn phase(&self) -> A::Phase {
        self.state.phase()
    }

    /// Run a command: enforce structure, decide, then evolve each emitted event.
    ///
    /// On a terminal phase or a kind mismatch the command is rejected before
    /// `decide` runs; on a domain rejection nothing is mutated. This is the
    /// in-memory loop for tests and trusted feeds; the persistent server loop
    /// (`execute`) appends to a journal between deciding and evolving.
    pub fn handle(
        &mut self,
        cmd: &A::Command,
        ctx: &mut A::Ctx,
    ) -> Result<Vec<A::Event>, Rejection<A>> {
        if let Some(rejection) = self.structural_check(cmd) {
            return Err(rejection);
        }
        let events = self.state.decide(cmd, ctx).map_err(Rejection::Domain)?;
        for event in &events {
            self.state.evolve(event);
        }
        Ok(events)
    }

    /// Apply a trusted event directly — for replay and trusted feeds.
    pub fn evolve(&mut self, event: &A::Event) {
        self.state.evolve(event);
    }

    /// Run the structural checks and `decide`, returning the events *without*
    /// evolving. The persistent loop (`execute`) uses this to append the events
    /// to a journal before applying them.
    pub fn decide_only(
        &self,
        cmd: &A::Command,
        ctx: &mut A::Ctx,
    ) -> Result<Vec<A::Event>, Rejection<A>> {
        if let Some(rejection) = self.structural_check(cmd) {
            return Err(rejection);
        }
        self.state.decide(cmd, ctx).map_err(Rejection::Domain)
    }

    /// The rejection `handle(cmd)` would return, or `None` if it would succeed.
    ///
    /// Runs the structural checks and `decide`, discarding any events. Call it
    /// with a `Ctx` whose entropy is a `probe` (e.g. `DeterministicCtx::probing`)
    /// so a speculative check never advances the journaled stream.
    pub fn why_not(&mut self, cmd: &A::Command, ctx: &mut A::Ctx) -> Option<Rejection<A>> {
        if let Some(rejection) = self.structural_check(cmd) {
            return Some(rejection);
        }
        match self.state.decide(cmd, ctx) {
            Ok(_) => None,
            Err(error) => Some(Rejection::Domain(error)),
        }
    }

    /// The terminal-phase and command-kind checks, shared by `handle`/`why_not`.
    fn structural_check(&self, cmd: &A::Command) -> Option<Rejection<A>> {
        let phase = self.state.phase();
        if phase.is_terminal() {
            return Some(Rejection::TerminalPhase { phase });
        }
        if let Some(expected) = phase.restriction()
            && !matches!(cmd.kinds(), Some(kinds) if kinds_intersect(expected, kinds))
        {
            return Some(Rejection::CommandKindRejected {
                phase,
                expected_kinds: expected,
            });
        }
        None
    }
}

/// Whether the command's kinds intersect the phase's accepted kinds.
fn kinds_intersect(expected: &[Kind], command: &[Kind]) -> bool {
    expected.iter().any(|kind| command.contains(kind))
}
