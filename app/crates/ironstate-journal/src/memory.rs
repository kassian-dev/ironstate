//! The reference in-memory journal: the implementation every storage adapter is
//! judged against by `journal_contract_test!`.

use crate::journal::{Journal, JournalError, Seq, Snapshot, VersionedEvent};
use ironstate_aggregate::{AggregateRules, DrawPos};
use std::borrow::Cow;

struct Record<A: AggregateRules> {
    events: Vec<A::Event>,
    entropy_pos: DrawPos,
}

/// An in-memory `Journal`. Construct it with the aggregate's genesis state (the
/// state before any append); it seeds a genesis snapshot at `Seq(0)` so replay
/// always has a base.
pub struct MemoryJournal<A: AggregateRules + Clone> {
    records: Vec<Record<A>>,
    snapshots: Vec<Snapshot<A>>,
}

impl<A: AggregateRules + Clone> MemoryJournal<A> {
    /// A fresh journal whose genesis (pre-append) state is `genesis`.
    pub fn new(genesis: A) -> Self {
        Self {
            records: Vec::new(),
            snapshots: vec![Snapshot {
                state: genesis,
                schema_version: 0,
                at: Seq(0),
                entropy_pos: DrawPos(0),
            }],
        }
    }

    fn record_at(&self, at: Seq) -> Result<&Record<A>, JournalError> {
        // Seq is public, so a caller can pass an out-of-range value. Compare in
        // u64 and only cast once it is within bounds — otherwise on a 32-bit
        // target an out-of-range Seq could truncate to a valid index instead of
        // returning UnknownSeq.
        if at.0 == 0 || at.0 > self.records.len() as u64 {
            return Err(JournalError::UnknownSeq { at });
        }
        Ok(&self.records[(at.0 - 1) as usize])
    }
}

/// Clone a snapshot without requiring `Snapshot: Clone` (which would force
/// `A: Clone` onto the whole `Journal` trait).
fn clone_snapshot<A: AggregateRules + Clone>(snapshot: &Snapshot<A>) -> Snapshot<A> {
    Snapshot {
        state: snapshot.state.clone(),
        schema_version: snapshot.schema_version,
        at: snapshot.at,
        entropy_pos: snapshot.entropy_pos,
    }
}

impl<A: AggregateRules + Clone> Journal<A> for MemoryJournal<A> {
    fn append(&mut self, events: &[A::Event], entropy_pos: DrawPos) -> Result<Seq, JournalError> {
        self.records.push(Record {
            events: events.to_vec(),
            entropy_pos,
        });
        Ok(Seq(self.records.len() as u64))
    }

    fn entropy_pos(&self, at: Seq) -> Result<DrawPos, JournalError> {
        if at.0 == 0 {
            // Genesis position. (A snapshot may also sit at Seq(0).)
            return Ok(self
                .snapshots
                .iter()
                .find(|s| s.at == Seq(0))
                .map_or(DrawPos(0), |s| s.entropy_pos));
        }
        Ok(self.record_at(at)?.entropy_pos)
    }

    fn head(&self) -> Option<Seq> {
        (!self.records.is_empty()).then_some(Seq(self.records.len() as u64))
    }

    fn events_since(&self, after: Option<Seq>) -> Result<Vec<VersionedEvent<A>>, JournalError> {
        // Saturate rather than truncate: an out-of-range `after` (possible only on
        // a 32-bit target, since Seq is public) means "past the end", so skip all.
        let start = after.map_or(0, |s| usize::try_from(s.0).unwrap_or(usize::MAX));
        let type_name = Cow::Borrowed(core::any::type_name::<A::Event>());
        Ok(self
            .records
            .iter()
            .skip(start)
            .flat_map(|record| record.events.iter())
            .map(|event| VersionedEvent {
                event: event.clone(),
                type_name: type_name.clone(),
                version: 1,
            })
            .collect())
    }

    fn snapshot(&mut self, snapshot: Snapshot<A>) -> Result<(), JournalError> {
        self.snapshots.push(snapshot);
        Ok(())
    }

    fn latest_snapshot(&self) -> Result<Option<Snapshot<A>>, JournalError> {
        // The highest-`at` snapshot — the most useful base for replay.
        Ok(self
            .snapshots
            .iter()
            .max_by_key(|s| s.at)
            .map(clone_snapshot))
    }

    fn fork(&self, at: Seq) -> Result<Self, JournalError> {
        if at.0 > self.records.len() as u64 {
            return Err(JournalError::UnknownSeq { at });
        }
        let cutoff = at.0 as usize;
        Ok(Self {
            records: self
                .records
                .iter()
                .take(cutoff)
                .map(|r| Record {
                    events: r.events.clone(),
                    entropy_pos: r.entropy_pos,
                })
                .collect(),
            snapshots: self
                .snapshots
                .iter()
                .filter(|s| s.at <= at)
                .map(clone_snapshot)
                .collect(),
        })
    }
}
