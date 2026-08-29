//! Replay, resume, the persistent `execute` loop, and the audit digest.

use crate::journal::{
    ExecuteError, Journal, JournalError, Seq, Snapshot, StreamId, VersionedEvent,
};
use ironstate::RestoreError;
use ironstate_aggregate::{
    Aggregate, AggregateRules, AuditDigest, CtxEntropy, DrawPos, Rejection, Seed, SeededEntropy,
    StableHash, audit_digest,
};

/// Rebuild an aggregate from a base snapshot and the events that followed it.
///
/// The aggregate is constructed past the initial phase (a snapshot is, by
/// right). The caller resumes live operation with entropy at `entropy_pos(head)`
/// — *not* `snapshot.entropy_pos` — which [`resume`] handles.
pub fn replay<A: AggregateRules>(
    snapshot: Snapshot<A>,
    events: &[VersionedEvent<A>],
) -> Result<Aggregate<A>, RestoreError> {
    let mut aggregate = Aggregate::from_state(snapshot.state);
    for stored in events {
        aggregate.evolve(&stored.event);
    }
    Ok(aggregate)
}

/// Rebuild an aggregate from the journal and the entropy stream positioned to
/// resume live operation.
///
/// The resume position is the one recorded **at the head**, because decides
/// between the latest snapshot and the head consumed draws — using
/// `snapshot.entropy_pos` instead is the canonical adapter bug.
///
/// ```ignore
/// // On startup: rebuild from the latest snapshot plus the events after it,
/// // with the entropy stream reopened at the head — ready to `execute` again.
/// let (aggregate, entropy) = resume(&journal, &stream, &seed)?;
/// ```
///
/// # Errors
///
/// Returns [`ResumeError::NoBase`] if the journal was never seeded with a
/// genesis snapshot, [`ResumeError::Journal`] if a read failed, or
/// [`ResumeError::Restore`] if a stored event or snapshot could not be upcast to
/// the current schema.
pub fn resume<A, J>(
    journal: &J,
    stream: &StreamId,
    seed: &Seed,
) -> Result<(Aggregate<A>, SeededEntropy), ResumeError>
where
    A: AggregateRules,
    J: Journal<A>,
{
    let snapshot = journal
        .latest_snapshot(stream)
        .map_err(ResumeError::Journal)?
        .ok_or(ResumeError::NoBase)?;
    let from = snapshot.at;
    let snapshot_pos = snapshot.entropy_pos;
    let events = journal
        .events_since(stream, Some(from))
        .map_err(ResumeError::Journal)?;

    let aggregate = replay(snapshot, &events).map_err(ResumeError::Restore)?;

    let resume_pos = match journal.head(stream) {
        Some(head) => journal
            .entropy_pos(stream, head)
            .map_err(ResumeError::Journal)?,
        None => snapshot_pos,
    };
    Ok((aggregate, SeededEntropy::at(seed, resume_pos)))
}

/// Why [`resume`] failed.
#[non_exhaustive]
#[derive(Debug)]
pub enum ResumeError {
    /// A journal read failed.
    Journal(crate::journal::JournalError),
    /// An event or snapshot failed to upcast.
    Restore(RestoreError),
    /// The journal has no base snapshot (it was not seeded with a genesis).
    NoBase,
}

impl core::fmt::Display for ResumeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Journal(error) => write!(f, "{error}"),
            Self::Restore(error) => write!(f, "{error}"),
            Self::NoBase => write!(
                f,
                "the journal has no base snapshot to replay from.\n\
                 A journal must be seeded with the aggregate's genesis state.\n\
                 Construct it via `MemoryJournal::new(genesis)` (or your adapter's equivalent).",
            ),
        }
    }
}

impl std::error::Error for ResumeError {}

/// The canonical persistent loop: structural checks → `decide` (draws) →
/// `append` (events + position, atomic) → `evolve` each event.
///
/// On any failure after draws were consumed, the live entropy stream is rewound
/// to the head position before returning, so a failed command leaves nothing
/// observable: no state change, no position change, no journal change.
///
/// This is a thin sandwich: [`prepare`] (the pure decide + position capture),
/// the journal's `append`, then [`Prepared::commit`] on success or
/// [`Prepared::abort`] on failure. A store that cannot implement the synchronous
/// [`Journal`] trait — an async one, say — reuses those same steps around its own
/// awaited append instead of copying this discipline. See [`prepare`].
///
/// # Errors
///
/// Returns [`ExecuteError::Rejected`] if the command never produced events (a
/// structural or domain rejection), or [`ExecuteError::Journal`] if the append
/// failed. Both leave the aggregate and the entropy stream where they started.
///
/// ```ignore
/// match execute(&mut journal, &stream, &mut aggregate, &cmd, &mut ctx) {
///     Ok(seq) => { /* durably appended at `seq`; the aggregate is up to date */ }
///     Err(ExecuteError::Rejected(why)) => { /* surface the rejection to the caller */ }
///     Err(ExecuteError::Journal(err)) => { /* storage failed; nothing changed */ }
/// }
/// ```
pub fn execute<A, J>(
    journal: &mut J,
    stream: &StreamId,
    aggregate: &mut Aggregate<A>,
    cmd: &A::Command,
    ctx: &mut A::Ctx,
) -> Result<Seq, ExecuteError<A>>
where
    A: AggregateRules,
    A::Ctx: CtxEntropy,
    J: for<'a> Journal<A, Tx<'a> = ()>,
{
    let mut unit = ();
    Ok(execute_in(journal, &mut unit, stream, aggregate, cmd, ctx)?.commit(aggregate))
}

/// The persistent loop for a journal that enlists in the **caller's** unit of
/// work: the transactional-outbox shape.
///
/// Identical to [`execute`] except that the append joins `tx`, and the
/// in-memory aggregate is **not** evolved. That last part is the whole point:
/// if the enclosing transaction is rolled back after the append, evolving
/// immediately would leave the aggregate silently ahead of the durable log.
/// So this hands back a [`Pending`], which the caller drives once the
/// transaction resolves:
///
/// ```ignore
/// let mut tx = pool.begin()?;
/// let pending = execute_in(&mut journal, &mut tx, &stream, &aggregate, &cmd, &mut ctx)?;
/// write_read_model(&mut tx, ...)?;   // the caller's other writes
/// enqueue_notice(&mut tx, ...)?;     // ... in the same transaction
/// match tx.commit() {
///     Ok(()) => { let seq = pending.commit(&mut aggregate); }
///     Err(e) => { pending.abort(&mut ctx); return Err(e.into()); }
/// }
/// ```
///
/// # One call per transaction
///
/// This reads the stream head through [`Journal::head`] / [`Journal::entropy_pos`],
/// which see **committed** state only — they take no `tx`. A second
/// `execute_in` against the same open transaction would therefore compute its
/// rewind anchor from the pre-transaction head, and aborting it would rewind the
/// entropy stream past the first append's draws, leaving a committed record
/// whose position is ahead of the live stream.
///
/// So: one `execute_in` per unit of work, unless your adapter's reads observe
/// its own uncommitted writes. Batching several commands atomically needs a
/// journal whose `head`/`entropy_pos` are transaction-aware, which the trait
/// does not yet express.
///
/// # Errors
///
/// Returns [`ExecuteError::Rejected`] if the command never produced events, or
/// [`ExecuteError::Journal`] if the append failed. Both leave the aggregate and
/// the entropy stream where they started.
pub fn execute_in<A, J>(
    journal: &mut J,
    tx: &mut J::Tx<'_>,
    stream: &StreamId,
    aggregate: &Aggregate<A>,
    cmd: &A::Command,
    ctx: &mut A::Ctx,
) -> Result<Pending<A>, ExecuteError<A>>
where
    A: AggregateRules,
    A::Ctx: CtxEntropy,
    J: Journal<A>,
{
    let head = head_pos(journal, stream).map_err(ExecuteError::Journal)?;
    let prepared = prepare(aggregate, cmd, ctx, head).map_err(ExecuteError::Rejected)?;

    match journal.append_in(tx, stream, prepared.events(), prepared.entropy_pos()) {
        Ok(seq) => Ok(Pending { seq, prepared }),
        Err(error) => {
            prepared.abort(ctx);
            Err(ExecuteError::Journal(error))
        }
    }
}

/// An append that reached the journal but whose enclosing unit of work has not
/// resolved yet.
///
/// Returned by [`execute_in`]. Call [`commit`](Self::commit) once the caller's
/// transaction commits, or [`abort`](Self::abort) if it rolls back — until one
/// of those happens the in-memory aggregate deliberately lags the log.
#[must_use = "a Pending must be committed or aborted once the enclosing transaction resolves"]
pub struct Pending<A: AggregateRules> {
    seq: Seq,
    prepared: Prepared<A>,
}

impl<A: AggregateRules> Pending<A> {
    /// The sequence the append landed at, valid once the transaction commits.
    pub fn seq(&self) -> Seq {
        self.seq
    }

    /// The enclosing transaction committed: advance the aggregate to match the
    /// durable log, and yield the sequence appended at.
    pub fn commit(self, aggregate: &mut Aggregate<A>) -> Seq {
        self.prepared.commit(aggregate);
        self.seq
    }

    /// The enclosing transaction rolled back: rewind the entropy stream so
    /// nothing is left observable. The aggregate was never evolved.
    pub fn abort(self, ctx: &mut A::Ctx)
    where
        A::Ctx: CtxEntropy,
    {
        self.prepared.abort(ctx);
    }
}

/// The entropy position recorded at the journal head, or `DrawPos(0)` if the
/// journal is empty. An async store computes the equivalent from its head row.
fn head_pos<A, J>(journal: &J, stream: &StreamId) -> Result<DrawPos, JournalError>
where
    A: AggregateRules,
    J: Journal<A>,
{
    match journal.head(stream) {
        Some(head) => journal.entropy_pos(stream, head),
        None => Ok(DrawPos(0)),
    }
}

/// A decided-but-not-yet-durable command: the events `decide` produced, the
/// entropy position they consumed, and the head position to rewind to if the
/// append fails.
///
/// Hold one across the append, then [`commit`](Self::commit) it on success or
/// [`abort`](Self::abort) it on failure. The append-before-evolve ordering, the
/// entropy-position capture, and the rewind-on-failure all live here, so a caller
/// — sync or async — supplies only the storage IO and cannot get the discipline
/// wrong.
#[must_use = "a Prepared must be committed or aborted, else the entropy stream is left advanced"]
pub struct Prepared<A: AggregateRules> {
    events: Vec<A::Event>,
    draws: DrawPos,
    head_pos: DrawPos,
}

impl<A: AggregateRules> Prepared<A> {
    /// The events to append, in order.
    pub fn events(&self) -> &[A::Event] {
        &self.events
    }

    /// The entropy position to persist **atomically** with the events.
    pub fn entropy_pos(&self) -> DrawPos {
        self.draws
    }

    /// Apply the events after the append succeeded, advancing the in-memory
    /// aggregate to match the durable log.
    pub fn commit(self, aggregate: &mut Aggregate<A>) {
        for event in &self.events {
            aggregate.evolve(event);
        }
    }

    /// Roll back after the append failed: rewind the entropy stream to the head
    /// position so nothing is left observable. The aggregate was never evolved,
    /// so it needs no change.
    pub fn abort(self, ctx: &mut A::Ctx)
    where
        A::Ctx: CtxEntropy,
    {
        rewind(ctx, self.head_pos);
    }
}

/// Run the pure half of the persistent loop: structural checks and `decide`,
/// capturing the entropy position the draws consumed — **without** touching the
/// journal or evolving the aggregate.
///
/// The caller supplies `head` (the position recorded at the journal head, or
/// `DrawPos(0)` for an empty journal) and performs the append between this and
/// [`Prepared::commit`] / [`Prepared::abort`]. [`execute`] is exactly this
/// sandwich around a synchronous `Journal::append`; an async store reuses the same
/// steps around its own awaited append:
///
/// ```ignore
/// let head = pg.head_pos(&stream).await.map_err(ExecuteError::Journal)?;
/// let prepared = prepare(&aggregate, cmd, ctx, head).map_err(ExecuteError::Rejected)?;
/// match pg.append(&stream, prepared.events(), prepared.entropy_pos()).await {
///     Ok(seq) => { prepared.commit(&mut aggregate); Ok(seq) }
///     Err(e)  => { prepared.abort(ctx); Err(ExecuteError::Journal(e)) }
/// }
/// ```
///
/// On rejection the entropy stream is rewound to `head` before returning, so a
/// rejected command leaves nothing observable.
pub fn prepare<A>(
    aggregate: &Aggregate<A>,
    cmd: &A::Command,
    ctx: &mut A::Ctx,
    head: DrawPos,
) -> Result<Prepared<A>, Rejection<A>>
where
    A: AggregateRules,
    A::Ctx: CtxEntropy,
{
    let events = match aggregate.decide_only(cmd, ctx) {
        Ok(events) => events,
        Err(rejection) => {
            rewind(ctx, head);
            return Err(rejection);
        }
    };
    let draws = ctx.entropy_mut().map_or(head, |entropy| entropy.draws());
    Ok(Prepared {
        events,
        draws,
        head_pos: head,
    })
}

fn rewind<C: CtxEntropy>(ctx: &mut C, pos: DrawPos) {
    if let Some(entropy) = ctx.entropy_mut() {
        entropy.seek(pos);
    }
}

/// Replay to the final state and return its collision-resistant audit digest.
///
/// This is the audit primitive: any holder of `(snapshot, events, revealed
/// seed)` recomputes and compares against a published `AuditDigest`.
pub fn replay_hash<A: AggregateRules + StableHash>(
    snapshot: Snapshot<A>,
    events: &[VersionedEvent<A>],
) -> Result<AuditDigest, RestoreError> {
    let aggregate = replay(snapshot, events)?;
    Ok(audit_digest(aggregate.state()))
}
