//! The reference in-memory journal: the implementation every storage adapter is
//! judged against by `journal_contract_test!`.

use crate::journal::{
    ForkableJournal, Journal, JournalError, RetainableJournal, Seq, Snapshot, StreamId,
    VersionedEvent,
};
use ironstate_aggregate::{AggregateRules, DrawPos};
use std::borrow::Cow;
use std::collections::BTreeMap;

struct Record<A: AggregateRules> {
    events: Vec<A::Event>,
    entropy_pos: DrawPos,
}

/// One stream's history: its records and the snapshots taken over them.
///
/// `truncated` counts the records dropped off the front, so a retained record's
/// `Seq` never changes when older ones are discarded — `records[i]` is always
/// `Seq(truncated + i + 1)`.
struct Stream<A: AggregateRules> {
    records: Vec<Record<A>>,
    snapshots: Vec<Snapshot<A>>,
    truncated: u64,
}

impl<A: AggregateRules> Stream<A> {
    fn new(genesis: Snapshot<A>) -> Self {
        Self {
            records: Vec::new(),
            snapshots: vec![genesis],
            truncated: 0,
        }
    }

    /// The latest sequence in this stream, or `None` if nothing was ever
    /// appended to it.
    ///
    /// A stream truncated all the way to its head still *has* a head — the
    /// records are gone, the history is not. Reporting `None` there would tell
    /// `execute` the stream is empty and rewind the live entropy stream to
    /// `DrawPos(0)`, so positions would run backwards while sequences ran
    /// forwards.
    fn head(&self) -> Option<Seq> {
        let head = self.truncated + self.records.len() as u64;
        (head > 0).then_some(Seq(head))
    }

    /// The earliest sequence still retained.
    fn retained_from(&self) -> Seq {
        Seq(self.truncated + 1)
    }

    fn record_at(&self, at: Seq) -> Result<&Record<A>, JournalError> {
        // Seq is public, so a caller can pass an out-of-range value. Compare in
        // u64 and only cast once it is within bounds — otherwise on a 32-bit
        // target an out-of-range Seq could truncate to a valid index instead of
        // returning UnknownSeq. A Seq below the retained horizon is equally
        // unknown: the record existed once, but this journal can no longer
        // answer for it.
        if at.0 <= self.truncated || at.0 > self.truncated + self.records.len() as u64 {
            return Err(JournalError::UnknownSeq { at });
        }
        Ok(&self.records[(at.0 - self.truncated - 1) as usize])
    }

    /// The newest snapshot's sequence, if the stream has one.
    fn latest_snapshot_at(&self) -> Option<Seq> {
        self.snapshots.iter().map(|s| s.at).max()
    }
}

/// An in-memory `Journal` over any number of streams. Construct it with the
/// aggregate's genesis state (the state before any append); every stream is
/// seeded with a genesis snapshot at `Seq(0)` on first use, so replay always has
/// a base.
///
/// Owns its own durability, so its unit of work is `()` and the
/// [`execute`](crate::execute) / [`resume`](crate::resume) helpers apply.
pub struct MemoryJournal<A: AggregateRules + Clone> {
    genesis: A,
    streams: BTreeMap<StreamId, Stream<A>>,
}

impl<A: AggregateRules + Clone> MemoryJournal<A> {
    /// A fresh journal whose streams each begin from the `genesis`
    /// (pre-append) state.
    pub fn new(genesis: A) -> Self {
        Self {
            genesis,
            streams: BTreeMap::new(),
        }
    }

    /// The genesis snapshot every stream starts from.
    fn genesis_snapshot(&self) -> Snapshot<A> {
        Snapshot {
            state: self.genesis.clone(),
            schema_version: 0,
            at: Seq(0),
            entropy_pos: DrawPos(0),
        }
    }

    /// The stream's history, seeded with its genesis snapshot if this is the
    /// first time it has been touched.
    fn stream_mut(&mut self, id: &StreamId) -> &mut Stream<A> {
        let genesis = self.genesis_snapshot();
        self.streams
            .entry(id.clone())
            .or_insert_with(|| Stream::new(genesis))
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
    type Tx<'a> = ();

    fn append_in(
        &mut self,
        _tx: &mut Self::Tx<'_>,
        stream: &StreamId,
        events: &[A::Event],
        entropy_pos: DrawPos,
    ) -> Result<Seq, JournalError> {
        let stream = self.stream_mut(stream);
        stream.records.push(Record {
            events: events.to_vec(),
            entropy_pos,
        });
        // Computed directly rather than via `head()`, which returns an Option
        // and would need unwrapping inside a public trait method.
        Ok(Seq(stream.truncated + stream.records.len() as u64))
    }

    fn snapshot_in(
        &mut self,
        _tx: &mut Self::Tx<'_>,
        stream: &StreamId,
        snapshot: Snapshot<A>,
    ) -> Result<(), JournalError> {
        self.stream_mut(stream).snapshots.push(snapshot);
        Ok(())
    }

    fn entropy_pos(&self, stream: &StreamId, at: Seq) -> Result<DrawPos, JournalError> {
        let Some(stream) = self.streams.get(stream) else {
            // An untouched stream is at its genesis and nowhere else.
            return if at.0 == 0 {
                Ok(DrawPos(0))
            } else {
                Err(JournalError::UnknownSeq { at })
            };
        };
        // A snapshot records the position at its own `Seq` and outlives the
        // record there, so after a truncation it may be the only thing that
        // still knows it — including at the head of a fully truncated stream.
        if let Some(snapshot) = stream.snapshots.iter().find(|s| s.at == at) {
            return Ok(snapshot.entropy_pos);
        }
        if at.0 == 0 {
            // Genesis is below the horizon once anything has been truncated, so
            // it is as unknown as any other discarded sequence — answering
            // `DrawPos(0)` here would fabricate a position for history that is
            // gone.
            if stream.truncated > 0 {
                return Err(JournalError::UnknownSeq { at });
            }
            return Ok(DrawPos(0));
        }
        Ok(stream.record_at(at)?.entropy_pos)
    }

    fn head(&self, stream: &StreamId) -> Option<Seq> {
        self.streams.get(stream)?.head()
    }

    fn events_since(
        &self,
        stream: &StreamId,
        after: Option<Seq>,
    ) -> Result<Vec<VersionedEvent<A>>, JournalError> {
        let Some(stream) = self.streams.get(stream) else {
            return Ok(Vec::new());
        };
        // Reading from below the horizon would silently return a *gapped* list:
        // the caller asked to continue from a point whose successors are partly
        // discarded. A subscription handed that list would drop records without
        // ever seeing an error, so refuse instead — `retained_from` says where a
        // valid read starts. `None` means "from this stream's start", which is
        // the horizon, so it is only valid on an untruncated stream.
        let from = after.map_or(0, |s| s.0);
        if from < stream.truncated {
            return Err(JournalError::UnknownSeq {
                at: after.unwrap_or(Seq(0)),
            });
        }
        // Saturate rather than truncate: an out-of-range `after` (possible only on
        // a 32-bit target, since Seq is public) means "past the end", so skip all.
        let start = usize::try_from(from - stream.truncated).unwrap_or(usize::MAX);
        let type_name = Cow::Borrowed(core::any::type_name::<A::Event>());
        Ok(stream
            .records
            .iter()
            .enumerate()
            .skip(start)
            .flat_map(|(i, record)| {
                // Every event in a record carries that record's Seq, not its own
                // index in the flattened list.
                let seq = Seq(stream.truncated + i as u64 + 1);
                record.events.iter().map(move |event| (seq, event))
            })
            .map(|(seq, event)| VersionedEvent {
                event: event.clone(),
                seq,
                type_name: type_name.clone(),
                version: 1,
            })
            .collect())
    }

    fn latest_snapshot(&self, stream: &StreamId) -> Result<Option<Snapshot<A>>, JournalError> {
        // An untouched stream still has a base to replay from: its genesis.
        let Some(stream) = self.streams.get(stream) else {
            return Ok(Some(self.genesis_snapshot()));
        };
        // The highest-`at` snapshot — the most useful base for replay.
        Ok(stream
            .snapshots
            .iter()
            .max_by_key(|s| s.at)
            .map(clone_snapshot))
    }
}

impl<A: AggregateRules + Clone> ForkableJournal<A> for MemoryJournal<A> {
    fn fork(&self, stream: &StreamId, at: Seq) -> Result<Self, JournalError> {
        let mut forked = Self::new(self.genesis.clone());
        let Some(source) = self.streams.get(stream) else {
            return if at.0 == 0 {
                Ok(forked)
            } else {
                Err(JournalError::UnknownSeq { at })
            };
        };
        // A fork must reproduce the source's records through `at`. Anything at
        // or below the truncation horizon is gone, so it cannot be forked —
        // and forking *at* the horizon would yield a branch whose head is not
        // the fork point.
        let head = source.truncated + source.records.len() as u64;
        let below_horizon = source.truncated > 0 && at.0 <= source.truncated;
        if at.0 > head || below_horizon {
            return Err(JournalError::UnknownSeq { at });
        }
        // A branch with no snapshot at or below the fork point cannot be
        // replayed at all. Truncation can discard the bases that covered this
        // point, so refuse here rather than hand back a journal whose `resume`
        // fails later with a less specific error.
        let base: Vec<Snapshot<A>> = source
            .snapshots
            .iter()
            .filter(|s| s.at <= at)
            .map(clone_snapshot)
            .collect();
        if base.is_empty() {
            return Err(JournalError::NoBaseForFork { at });
        }

        let cutoff = (at.0 - source.truncated) as usize;
        forked.streams.insert(
            stream.clone(),
            Stream {
                truncated: source.truncated,
                records: source
                    .records
                    .iter()
                    .take(cutoff)
                    .map(|r| Record {
                        events: r.events.clone(),
                        entropy_pos: r.entropy_pos,
                    })
                    .collect(),
                snapshots: base,
            },
        );
        Ok(forked)
    }
}

impl<A: AggregateRules + Clone> RetainableJournal<A> for MemoryJournal<A> {
    fn truncate_before(&mut self, id: &StreamId, at: Seq) -> Result<(), JournalError> {
        // Look up read-only first: a refused truncation must leave the journal
        // exactly as it was, and `stream_mut` would create the stream — so a
        // typo'd id would leave a phantom entry behind that `streams()` reports.
        let Some(stream) = self.streams.get(id) else {
            return Err(JournalError::UnknownSeq { at });
        };

        // `at` may sit one past the head (discard everything) but no further.
        // Without this a snapshot recorded beyond the head would authorise an
        // arbitrary truncation, and `retained_from` would then disagree with the
        // `at` that was asked for.
        let head = stream.truncated + stream.records.len() as u64;
        if at.0 > head + 1 {
            return Err(JournalError::UnknownSeq { at });
        }

        let latest_snapshot = stream.latest_snapshot_at();

        // Replay resumes from the newest snapshot and reads every record after
        // it, so the retained prefix must start at or before that point.
        let needed = at.0.saturating_sub(1);
        if latest_snapshot.is_none_or(|s| s.0 < needed) {
            return Err(JournalError::NoSnapshotForTruncation {
                at,
                latest_snapshot,
            });
        }

        // Re-fetch mutably now the read-only checks have passed. The lookup
        // cannot fail here, but saying so with the same error the read phase
        // would have returned keeps this method panic-free.
        let Some(stream) = self.streams.get_mut(id) else {
            return Err(JournalError::UnknownSeq { at });
        };

        let drop_count = at.0.saturating_sub(1).saturating_sub(stream.truncated);
        let drop_count = usize::try_from(drop_count)
            .unwrap_or(usize::MAX)
            .min(stream.records.len());
        stream.records.drain(..drop_count);
        stream.truncated += drop_count as u64;

        // A snapshot below the new horizon is no longer a valid base: replaying
        // from it would need records that are now gone, and would silently
        // produce a *wrong* aggregate rather than an error. The refusal check
        // above guarantees at least one snapshot survives this.
        let horizon = stream.truncated;
        stream.snapshots.retain(|s| s.at.0 >= horizon);
        Ok(())
    }

    fn retained_from(&self, stream: &StreamId) -> Seq {
        self.streams
            .get(stream)
            .map_or(Seq(1), Stream::retained_from)
    }

    fn streams(&self) -> Result<Vec<StreamId>, JournalError> {
        Ok(self.streams.keys().cloned().collect())
    }
}
