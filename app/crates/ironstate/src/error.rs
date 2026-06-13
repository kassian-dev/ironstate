//! Structured rejection and restore errors.

use crate::kind::Kind;
use core::fmt;

/// Why a transition was rejected.
///
/// The rejected event moves *into* the error so a caller that wants to retry,
/// log, or re-route it gets it back without a clone; the state is cloned from
/// `&self`. The `Display` form is teaching prose (what happened, why, and what
/// to do); the typed fields let a consumer branch on the cause directly.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionError<S, E> {
    /// The machine is in a terminal state, which has no outbound transitions.
    TerminalState {
        /// The terminal state the machine was in.
        state: S,
        /// The event that was rejected.
        event: E,
    },
    /// The event's kind is not among the kinds this state will accept.
    EventKindRejected {
        /// The state that rejected the event.
        state: S,
        /// The event that was rejected.
        event: E,
        /// The kinds this state accepts.
        expected_kinds: &'static [Kind],
        /// The kinds the event carries, or `None` for the default kind.
        event_kind: Option<&'static [Kind]>,
    },
    /// The transition function returned `None` for this state/event pair.
    NoTransition {
        /// The state the machine was in.
        state: S,
        /// The event that was rejected.
        event: E,
    },
}

impl<S, E> TransitionError<S, E> {
    /// The state the machine was in when the event was rejected.
    pub fn state(&self) -> &S {
        match self {
            Self::TerminalState { state, .. }
            | Self::EventKindRejected { state, .. }
            | Self::NoTransition { state, .. } => state,
        }
    }

    /// Borrow the rejected event.
    pub fn event(&self) -> &E {
        match self {
            Self::TerminalState { event, .. }
            | Self::EventKindRejected { event, .. }
            | Self::NoTransition { event, .. } => event,
        }
    }

    /// Recover the rejected event by value.
    pub fn into_event(self) -> E {
        match self {
            Self::TerminalState { event, .. }
            | Self::EventKindRejected { event, .. }
            | Self::NoTransition { event, .. } => event,
        }
    }
}

impl<S: fmt::Debug, E: fmt::Debug> fmt::Display for TransitionError<S, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TerminalState { state, event } => write!(
                f,
                "event {event:?} was rejected because {state:?} is a terminal state.\n\
                 Terminal states have no outbound transitions.\n\
                 To leave this state you must start a new machine or model the recovery \
                 as a non-terminal state.",
            ),
            Self::EventKindRejected {
                state,
                event,
                expected_kinds,
                event_kind,
            } => write!(
                f,
                "event {event:?} was rejected by {state:?} because of an event-kind \
                 mismatch.\n\
                 {state:?} only accepts events of kind {expected_kinds:?}, but {event:?} \
                 carries kind {event_kind:?}.\n\
                 Annotate the event with a matching `#[event_kind = …]`, or relax the \
                 state's `#[only_accepts(kind = …)]`.",
            ),
            Self::NoTransition { state, event } => write!(
                f,
                "event {event:?} was rejected because no transition is defined from \
                 {state:?}.\n\
                 The transition function returned `None` for this state/event pair.\n\
                 Add a match arm `({state:?}, {event:?}) => Some(…)` if this transition \
                 should be legal.",
            ),
        }
    }
}

impl<S: fmt::Debug, E: fmt::Debug> std::error::Error for TransitionError<S, E> {}

/// Why a versioned restore failed.
///
/// Every failure is typed and surfaces at the load boundary — a stream written
/// by a newer binary, an unknown version tag, or a payload that would not
/// decode — never as a raw deserialization panic.
#[non_exhaustive]
#[derive(Debug)]
pub enum RestoreError {
    /// The stored version is newer than this binary understands.
    NewerThanBinary {
        /// The version found in the stored envelope.
        found: u32,
        /// The highest version this binary supports.
        supports: u32,
    },
    /// The stored version is below 1 or absent from the migration history.
    UnknownVersion {
        /// The version found in the stored envelope.
        found: u32,
    },
    /// The payload for a known version failed to decode.
    Decode {
        /// The version whose payload failed to decode.
        version: u32,
        /// The underlying decode error.
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

impl fmt::Display for RestoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NewerThanBinary { found, supports } => write!(
                f,
                "stored version {found} is newer than this binary supports \
                 (max {supports}).\n\
                 The data was written by a newer release.\n\
                 Upgrade the binary, or migrate the data down before loading it here.",
            ),
            Self::UnknownVersion { found } => write!(
                f,
                "stored version {found} is not in this type's migration history.\n\
                 Versions start at 1 and must be listed in `history` to be loadable.\n\
                 Check the envelope was written by this type, and that `history` \
                 covers every version that may still exist on disk.",
            ),
            Self::Decode { version, source } => write!(
                f,
                "the payload for version {version} failed to decode: {source}.\n\
                 The bytes did not match the type recorded for that version.\n\
                 Confirm the serde format matches the one used to write the data.",
            ),
        }
    }
}

impl std::error::Error for RestoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Decode { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}
