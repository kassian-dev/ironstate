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

    /// The latest sequence in this stream, or `None` if it holds no records.
    fn head(&self) -> Option<Seq> {
        (!self.records.is_empty()).then_some(Seq(self.truncated + self.records.len() as u64))
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
        if !self.streams.contains_key(id) {
            let seeded = Stream::new(self.genesis_snapshot());
            self.streams.insert(id.clone(), seeded);
        }
        self.streams
            .get_mut(id)
            .expect("the stream was just inserted")
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
        Ok(stream.head().expect("a record was just pushed"))
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
        if at.0 == 0 {
            // Genesis position. (A snapshot may also sit at Seq(0).)
            return Ok(stream
                .snapshots
                .iter()
                .find(|s| s.at == Seq(0))
                .map_or(DrawPos(0), |s| s.entropy_pos));
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
        // Saturate rather than truncate: an out-of-range `after` (possible only on
        // a 32-bit target, since Seq is public) means "past the end", so skip all.
        // `after` is an absolute Seq, so discount whatever was truncated away.
        let start = after.map_or(0, |s| {
            usize::try_from(s.0.saturating_sub(stream.truncated)).unwrap_or(usize::MAX)
        });
        let type_name = Cow::Borrowed(core::any::type_name::<A::Event>());
        Ok(stream
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
                snapshots: source
                    .snapshots
                    .iter()
                    .filter(|s| s.at <= at)
                    .map(clone_snapshot)
                    .collect(),
            },
        );
        Ok(forked)
    }
}

impl<A: AggregateRules + Clone> RetainableJournal<A> for MemoryJournal<A> {
    fn truncate_before(&mut self, stream: &StreamId, at: Seq) -> Result<(), JournalError> {
        let stream = self.stream_mut(stream);
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
}
