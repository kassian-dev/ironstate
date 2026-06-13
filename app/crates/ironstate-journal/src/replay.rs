//! Replay, resume, the persistent `execute` loop, and the audit digest.

use crate::journal::{ExecuteError, Journal, Seq, Snapshot, VersionedEvent};
use ironstate::RestoreError;
use ironstate_aggregate::{
    Aggregate, AggregateRules, AuditDigest, CtxEntropy, DrawPos, Seed, SeededEntropy, StableHash,
    audit_digest,
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
pub fn resume<A, J>(journal: &J, seed: &Seed) -> Result<(Aggregate<A>, SeededEntropy), ResumeError>
where
    A: AggregateRules,
    J: Journal<A>,
{
    let snapshot = journal
        .latest_snapshot()
        .map_err(ResumeError::Journal)?
        .ok_or(ResumeError::NoBase)?;
    let from = snapshot.at;
    let snapshot_pos = snapshot.entropy_pos;
    let events = journal
        .events_since(Some(from))
        .map_err(ResumeError::Journal)?;

    let aggregate = replay(snapshot, &events).map_err(ResumeError::Restore)?;

    let resume_pos = match journal.head() {
        Some(head) => journal.entropy_pos(head).map_err(ResumeError::Journal)?,
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
pub fn execute<A, J>(
    journal: &mut J,
    aggregate: &mut Aggregate<A>,
    cmd: &A::Command,
    ctx: &mut A::Ctx,
) -> Result<Seq, ExecuteError<A>>
where
    A: AggregateRules,
    A::Ctx: CtxEntropy,
    J: Journal<A>,
{
    let head_pos = match journal.head() {
        Some(head) => journal.entropy_pos(head).map_err(ExecuteError::Journal)?,
        None => DrawPos(0),
    };

    let events = match aggregate.decide_only(cmd, ctx) {
        Ok(events) => events,
        Err(rejection) => {
            rewind(ctx, head_pos);
            return Err(ExecuteError::Rejected(rejection));
        }
    };

    // The position after decide consumed its draws (or the head position for an
    // entropy-free context).
    let draws = ctx
        .entropy_mut()
        .map_or(head_pos, |entropy| entropy.draws());

    match journal.append(&events, draws) {
        Ok(seq) => {
            for event in &events {
                aggregate.evolve(event);
            }
            Ok(seq)
        }
        Err(error) => {
            rewind(ctx, head_pos);
            Err(ExecuteError::Journal(error))
        }
    }
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
