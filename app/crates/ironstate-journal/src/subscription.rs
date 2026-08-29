//! Subscriptions: a process-manager pattern with idempotent delivery.
//!
//! This is a pattern with tests, not a message bus — durable transport is the
//! consumer's outbox. Idempotency keys are `(StreamId, Seq)` composites, since
//! `Seq` alone is only unique per stream.

use crate::journal::{ExecuteError, Journal, Seq, StreamId};
use crate::replay::execute;
use ironstate_aggregate::{Aggregate, AggregateRules, CtxEntropy};
use std::collections::BTreeMap;
use std::marker::PhantomData;

/// One event read from a source stream: which stream it came from, where in that
/// stream it sits, and the event itself.
///
/// These three always travel together — the position is only meaningful
/// alongside the stream it indexes — so they are one argument rather than three.
pub struct SourceEvent<'a, F: AggregateRules> {
    /// The stream the event was read from.
    pub stream: &'a StreamId,
    /// The event's position within that stream.
    pub at: Seq,
    /// The event itself.
    pub event: &'a F::Event,
}

/// The outcome of delivering one source event to a subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivered {
    /// The event advanced the target and the high-water mark.
    Applied,
    /// The event was at or below the recorded mark, so it was dropped.
    Duplicate,
}

/// A target aggregate that reacts to a source aggregate's events by issuing
/// commands to itself.
pub trait React<F: AggregateRules>: AggregateRules {
    /// The commands to apply in response to `event` at `at`.
    fn react(&self, event: &F::Event, at: Seq) -> Vec<Self::Command>;
}

/// Delivers a source stream's events to a target aggregate exactly once,
/// tracking a per-stream high-water mark.
pub struct Subscription<F: AggregateRules, T: React<F>> {
    marks: BTreeMap<StreamId, Seq>,
    _marker: PhantomData<fn(F, T)>,
}

impl<F: AggregateRules, T: React<F>> Default for Subscription<F, T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<F: AggregateRules, T: React<F>> Subscription<F, T> {
    /// A subscription with no marks yet.
    pub fn new() -> Self {
        Self {
            marks: BTreeMap::new(),
            _marker: PhantomData,
        }
    }

    /// The high-water mark recorded for a stream, if any.
    pub fn mark(&self, stream: &StreamId) -> Option<Seq> {
        self.marks.get(stream).copied()
    }

    /// Deliver one source event.
    ///
    /// At or below the recorded mark for `source`, the event is a duplicate and
    /// is dropped (`Ok(Duplicate)`). Above it, the target reacts and each command
    /// is `execute`d against `target_stream` in the target journal; the mark only
    /// advances once all of them have committed — so a failure mid-delivery leaves
    /// the mark where it was and the event is retried on redelivery.
    ///
    /// The source stream and `target_stream` are independent: the first names the
    /// history being read, the second the history being written.
    ///
    /// # Errors
    ///
    /// Returns whatever [`execute`] returned for the first command that failed;
    /// the high-water mark is left unadvanced.
    pub fn deliver<J>(
        &mut self,
        source: SourceEvent<'_, F>,
        target_stream: &StreamId,
        target: &mut Aggregate<T>,
        ctx: &mut T::Ctx,
        journal: &mut J,
    ) -> Result<Delivered, ExecuteError<T>>
    where
        T::Ctx: CtxEntropy,
        J: for<'a> Journal<T, Tx<'a> = ()>,
    {
        let SourceEvent { stream, at, event } = source;
        if let Some(mark) = self.marks.get(stream)
            && at <= *mark
        {
            return Ok(Delivered::Duplicate);
        }

        let commands = <T as React<F>>::react(target.state(), event, at);
        for command in &commands {
            execute(journal, target_stream, target, command, ctx)?;
        }
        self.marks.insert(stream.clone(), at);
        Ok(Delivered::Applied)
    }
}
