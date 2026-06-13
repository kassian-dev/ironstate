//! The journal trait and the records it stores.

use ironstate_aggregate::{AggregateRules, DrawPos, Rejection};
use std::borrow::Cow;

/// A position in a stream: monotonic, 1-based. `Seq(0)` is the genesis (the
/// state before any append).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Seq(pub u64);

/// A point-in-time state, captured so replay need not start from the genesis.
///
/// Carries the entropy position at its own `Seq`, but note: the authoritative
/// resume position after replay is the one recorded *at the head*, not here —
/// decides between this snapshot and the head consumed draws.
pub struct Snapshot<A: AggregateRules> {
    /// The captured state.
    pub state: A,
    /// The state schema version this snapshot was written with.
    pub schema_version: u32,
    /// The sequence number this snapshot was taken at.
    pub at: Seq,
    /// The entropy position at `at`.
    pub entropy_pos: DrawPos,
}

/// A stored event tagged with the type and version it was written as, so a
/// mixed-version stream can be upcast per event at load.
pub struct VersionedEvent<A: AggregateRules> {
    /// The event payload.
    pub event: A::Event,
    /// The event type's name when stored.
    pub type_name: Cow<'static, str>,
    /// The event enum's version when stored.
    pub version: u32,
}

/// A failure from the storage layer.
#[non_exhaustive]
#[derive(Debug)]
pub enum JournalError {
    /// The underlying store failed.
    Storage(Box<dyn std::error::Error + Send + Sync>),
    /// No record exists at the requested sequence number.
    UnknownSeq {
        /// The sequence number that was not found.
        at: Seq,
    },
}

impl core::fmt::Display for JournalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Storage(source) => write!(f, "journal storage error: {source}"),
            Self::UnknownSeq { at } => write!(
                f,
                "no record at sequence {at:?}.\n\
                 The sequence is past the head, or below the earliest retained record.\n\
                 Check `head()` and the snapshot horizon before addressing a Seq.",
            ),
        }
    }
}

impl std::error::Error for JournalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(source) => Some(source.as_ref()),
            Self::UnknownSeq { .. } => None,
        }
    }
}

/// Why `execute` failed. On either variant nothing was journaled, nothing was
/// mutated, and the entropy stream was rewound to the head position.
#[non_exhaustive]
pub enum ExecuteError<A: AggregateRules> {
    /// The command was rejected before anything was journaled.
    Rejected(Rejection<A>),
    /// The append to the journal failed.
    Journal(JournalError),
}

impl<A: AggregateRules> core::fmt::Debug for ExecuteError<A> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Rejected(rejection) => f.debug_tuple("Rejected").field(rejection).finish(),
            Self::Journal(error) => f.debug_tuple("Journal").field(error).finish(),
        }
    }
}

impl<A: AggregateRules> core::fmt::Display for ExecuteError<A> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Rejected(rejection) => write!(f, "{rejection}"),
            Self::Journal(error) => write!(f, "{error}"),
        }
    }
}

impl<A: AggregateRules> std::error::Error for ExecuteError<A> {}

/// An append-only log of an aggregate's events, paired with the entropy position
/// each append consumed.
///
/// The load-bearing rule: `append` MUST persist the events and their entropy
/// position in one atomic unit, because replay cannot recompute the position
/// from the events.
pub trait Journal<A: AggregateRules> {
    /// Append a batch of events with the entropy position consumed producing
    /// them, returning the new head sequence. Atomic.
    fn append(&mut self, events: &[A::Event], entropy_pos: DrawPos) -> Result<Seq, JournalError>;

    /// The entropy position recorded at `at`.
    fn entropy_pos(&self, at: Seq) -> Result<DrawPos, JournalError>;

    /// The latest sequence number, or `None` if nothing has been appended.
    fn head(&self) -> Option<Seq>;

    /// Every event after `after` (or from the start if `None`), in order.
    fn events_since(&self, after: Option<Seq>) -> Result<Vec<VersionedEvent<A>>, JournalError>;

    /// Store a snapshot.
    fn snapshot(&mut self, snapshot: Snapshot<A>) -> Result<(), JournalError>;

    /// The most recent snapshot, if any.
    fn latest_snapshot(&self) -> Result<Option<Snapshot<A>>, JournalError>;

    /// A logically independent journal whose records through `at` are identical,
    /// including the entropy position at `at`.
    fn fork(&self, at: Seq) -> Result<Self, JournalError>
    where
        Self: Sized;
}
