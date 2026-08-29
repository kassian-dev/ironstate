//! The journal trait and the records it stores.

use ironstate_aggregate::{AggregateRules, DrawPos, Rejection};
use std::borrow::Cow;

/// A position **within one stream**: monotonic, 1-based. `Seq(0)` is that
/// stream's genesis (the state before any append to it).
///
/// Sequences are per-stream, not global: `Seq(4)` in one stream is unrelated to
/// `Seq(4)` in another. This matches the `(StreamId, Seq)` key
/// [`Subscription`](crate::Subscription) already uses for idempotency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Seq(pub u64);

/// Identifies one stream — one aggregate instance's history — within a journal.
///
/// One journal value holds many streams. This is the same identifier
/// [`Subscription`](crate::Subscription) keys its high-water marks by, so the
/// read and write sides address history the same way.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct StreamId(pub String);

impl StreamId {
    /// A stream id from anything string-like.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl core::fmt::Display for StreamId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for StreamId {
    fn from(id: &str) -> Self {
        Self(id.to_owned())
    }
}

impl From<String> for StreamId {
    fn from(id: String) -> Self {
        Self(id)
    }
}

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
#[non_exhaustive]
pub struct VersionedEvent<A: AggregateRules> {
    /// The event payload.
    pub event: A::Event,
    /// The sequence of the **record** this event was appended in.
    ///
    /// One record can hold several events, so this is not a per-event ordinal
    /// and several returned events may share it. It is the identity a
    /// [`Subscription`](crate::Subscription) keys its high-water mark by, and
    /// the only correct thing to pass as
    /// [`SourceEvent::at`](crate::SourceEvent::at) — a position derived from the
    /// index within the returned list is wrong as soon as any `decide` emits
    /// more than one event.
    pub seq: Seq,
    /// The event type's name when stored.
    pub type_name: Cow<'static, str>,
    /// The event enum's version when stored.
    pub version: u32,
}

impl<A: AggregateRules> VersionedEvent<A> {
    /// A stored event, tagged with the record it came from and the schema it
    /// was written under.
    ///
    /// Adapters build these in `events_since`. The struct is
    /// `#[non_exhaustive]`, so it is constructed through here rather than by
    /// literal: a future field is then an additive change for every adapter
    /// instead of a breaking one.
    pub fn new(
        event: A::Event,
        seq: Seq,
        type_name: impl Into<Cow<'static, str>>,
        version: u32,
    ) -> Self {
        Self {
            event,
            seq,
            type_name: type_name.into(),
            version,
        }
    }
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
    /// A fork was refused: no snapshot at or below the fork point survives, so
    /// the branch would have no base to replay from.
    NoBaseForFork {
        /// The requested fork point.
        at: Seq,
    },
    /// Truncation was refused: it would have discarded records that are still
    /// needed to replay from the newest snapshot.
    NoSnapshotForTruncation {
        /// The sequence truncation was requested before.
        at: Seq,
        /// The newest snapshot in the stream, if it has one at all.
        latest_snapshot: Option<Seq>,
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
                 Check `head(stream)` for the upper bound, and — on a journal that \
                 truncates — `RetainableJournal::retained_from(stream)` for the lower \
                 one, before addressing a Seq.",
            ),
            Self::NoBaseForFork { at } => write!(
                f,
                "cannot fork at {at:?}: no snapshot at or below it survives.\n\
                 Truncation discarded the bases that covered this point, so the branch \
                 would be unresumable.\n\
                 Fork at or above the newest snapshot instead, or take one first.",
            ),
            Self::NoSnapshotForTruncation {
                at,
                latest_snapshot,
            } => write!(
                f,
                "refusing to truncate before {at:?}: the newest snapshot is {latest_snapshot:?}.\n\
                 Truncating here would discard records still needed to replay from that \
                 snapshot, leaving the stream unresumable.\n\
                 Take a snapshot at or after {:?} first, then truncate.",
                Seq(at.0.saturating_sub(1)),
            ),
        }
    }
}

impl std::error::Error for JournalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(source) => Some(source.as_ref()),
            Self::UnknownSeq { .. }
            | Self::NoBaseForFork { .. }
            | Self::NoSnapshotForTruncation { .. } => None,
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

/// An append-only log of aggregate events, paired with the entropy position
/// each append consumed, held across one or more independent streams.
///
/// The load-bearing rule: an append MUST persist the events and their entropy
/// position in one atomic unit, because replay cannot recompute the position
/// from the events.
///
/// # Streams
///
/// One journal value holds many streams, each its own aggregate instance with
/// its own [`Seq`] line, snapshots and entropy positions. Appending to one
/// stream never moves another's head — a property
/// [`journal_contract_test!`](crate::journal_contract_test) enforces.
///
/// # Units of work
///
/// [`Tx`](Self::Tx) is the caller-supplied unit of work an append enlists in.
/// A journal that owns its own durability sets it to `()`; a journal living in
/// the same database as the caller's read models sets it to that database's
/// transaction, so the append and the caller's other writes commit together or
/// not at all.
///
/// When `Tx<'_>` is `()`, use the [`execute`](crate::execute) /
/// [`resume`](crate::resume) helpers, which hide it entirely. Otherwise use
/// [`execute_in`](crate::execute_in) and drive the returned
/// [`Pending`](crate::Pending) once your transaction resolves.
pub trait Journal<A: AggregateRules> {
    /// The caller-supplied unit of work an append enlists in.
    ///
    /// Set this to `()` when the journal owns its own durability.
    type Tx<'a>;

    /// Append a batch of events to `stream`, with the entropy position consumed
    /// producing them, returning the stream's new head sequence.
    ///
    /// The events and the position must land atomically, and — if `Tx` is a real
    /// transaction — must not be visible until that transaction commits.
    ///
    /// # Errors
    ///
    /// Returns [`JournalError::Storage`] if the underlying store failed.
    fn append_in(
        &mut self,
        tx: &mut Self::Tx<'_>,
        stream: &StreamId,
        events: &[A::Event],
        entropy_pos: DrawPos,
    ) -> Result<Seq, JournalError>;

    /// Store a snapshot of `stream`, enlisting in the same unit of work.
    ///
    /// # Errors
    ///
    /// Returns [`JournalError::Storage`] if the underlying store failed.
    fn snapshot_in(
        &mut self,
        tx: &mut Self::Tx<'_>,
        stream: &StreamId,
        snapshot: Snapshot<A>,
    ) -> Result<(), JournalError>;

    /// The entropy position recorded in `stream` at `at`.
    ///
    /// # Errors
    ///
    /// Returns [`JournalError::UnknownSeq`] if `at` is past the stream's head or
    /// below its retained horizon — never a different record's position.
    fn entropy_pos(&self, stream: &StreamId, at: Seq) -> Result<DrawPos, JournalError>;

    /// The latest sequence number in `stream`, or `None` if it holds nothing.
    fn head(&self, stream: &StreamId) -> Option<Seq>;

    /// Every event in `stream` after `after` (or from its start if `None`), in
    /// order.
    ///
    /// # Errors
    ///
    /// Returns [`JournalError::UnknownSeq`] if `after` is below the stream's
    /// retained horizon, since the result would otherwise have a silent gap in
    /// it. That is a stale mark on a stream some of whose history has been
    /// truncated away — a stream that simply has no history yet is not below
    /// anything, and reads from it (including `None`) succeed with an empty
    /// list.
    ///
    /// `None` means genesis, so on a *truncated* stream it is itself below the
    /// horizon and refused; ask for the record before the horizon instead.
    /// Returns [`JournalError::Storage`] if the underlying store failed.
    fn events_since(
        &self,
        stream: &StreamId,
        after: Option<Seq>,
    ) -> Result<Vec<VersionedEvent<A>>, JournalError>;

    /// The most recent snapshot of `stream`, if any.
    ///
    /// # Errors
    ///
    /// Returns [`JournalError::Storage`] if the underlying store failed.
    fn latest_snapshot(&self, stream: &StreamId) -> Result<Option<Snapshot<A>>, JournalError>;
}

/// A journal that can branch a stream's history at a point.
///
/// Forking is essential for simulation and for the game-shaped workloads the
/// family targets — [`scenario_test!`](crate::scenario_test) requires it — but
/// it is meaningless, and sometimes forbidden, for a store of statutory records.
/// It is therefore a capability an adapter opts into rather than part of every
/// journal's contract: a relational adapter that would have to copy rows to
/// satisfy a method it never calls can simply not implement this.
pub trait ForkableJournal<A: AggregateRules>: Journal<A> {
    /// A logically independent journal whose records for `stream` through `at`
    /// are identical, including the entropy position at `at`.
    ///
    /// # Errors
    ///
    /// Returns [`JournalError::UnknownSeq`] if `at` is past the stream's head
    /// or at or below its retained horizon, and
    /// [`JournalError::NoBaseForFork`] if no snapshot at or below `at`
    /// survives — the branch would have nothing to replay from.
    fn fork(&self, stream: &StreamId, at: Seq) -> Result<Self, JournalError>
    where
        Self: Sized;
}

/// A journal that can discard the oldest part of a stream.
///
/// Retention is a capability, not a universal contract: an append-only store
/// may have no way to drop records, and some domains forbid it outright. It is
/// therefore opt-in — but where it exists, anyone with a retention policy, a
/// right-to-erasure obligation, or simply a large log needs it, and the
/// snapshot machinery that makes it safe already exists.
pub trait RetainableJournal<A: AggregateRules>: Journal<A> {
    /// Discard every record in `stream` before `at`, keeping `at` onward.
    ///
    /// Truncation must be refused unless a snapshot at or after `at - 1`
    /// exists, since replay resumes from the newest snapshot and would
    /// otherwise need records that are gone. Sequence numbers of the *retained*
    /// records never change: a `Seq` is an identity, and subscriptions hold
    /// high-water marks that refer to it.
    ///
    /// # Errors
    ///
    /// Returns [`JournalError::UnknownSeq`] if `at` is not a truncation point
    /// this stream can express — `Seq(0)` is genesis rather than a record, `at`
    /// beyond one past the head would discard more than exists, and an unknown
    /// stream has nothing to truncate. Returns
    /// [`JournalError::NoSnapshotForTruncation`] if no snapshot covers the
    /// retained prefix, or [`JournalError::Storage`] if the store failed.
    fn truncate_before(&mut self, stream: &StreamId, at: Seq) -> Result<(), JournalError>;

    /// The earliest sequence still retained in `stream` — everything below it
    /// has been truncated away.
    fn retained_from(&self, stream: &StreamId) -> Seq;

    /// Every stream this journal holds, so a retention sweep can enumerate what
    /// it might expire.
    ///
    /// The order is unspecified — treat the result as a set. An adapter is free
    /// to return rows in whatever order its store yields them, and callers that
    /// need a stable order should sort.
    ///
    /// This lives here rather than on [`Journal`] because sweeping is the only
    /// thing that needs it, and on a relational store it is a full-table scan
    /// no ordinary adapter should be made to implement.
    ///
    /// # Errors
    ///
    /// Returns [`JournalError::Storage`] if the underlying store failed.
    fn streams(&self) -> Result<Vec<StreamId>, JournalError>;
}
