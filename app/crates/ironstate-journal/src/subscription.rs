//! Subscriptions: a process-manager pattern with idempotent delivery.
//!
//! This is a pattern with tests, not a message bus — durable transport is the
//! consumer's outbox. Idempotency keys are `(StreamId, Seq)` composites, since
//! `Seq` alone is only unique per stream.

use crate::journal::{ExecuteError, Journal, Seq};
use crate::replay::execute;
use ironstate_aggregate::{Aggregate, AggregateRules, CtxEntropy};
use std::collections::BTreeMap;
use std::marker::PhantomData;

/// Identifies one source stream, so per-stream high-water marks stay distinct.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StreamId(pub String);

impl StreamId {
    /// A stream id from anything string-like.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
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
    /// At or below the recorded mark for `stream`, the event is a duplicate and
    /// is dropped (`Ok(Duplicate)`). Above it, the target reacts and each command
    /// is `execute`d against the target journal; the mark only advances once all
    /// of them have committed — so a failure mid-delivery leaves the mark where
    /// it was and the event is retried on redelivery.
    pub fn deliver<J: Journal<T>>(
        &mut self,
        stream: &StreamId,
        at: Seq,
        event: &F::Event,
        target: &mut Aggregate<T>,
        ctx: &mut T::Ctx,
        journal: &mut J,
    ) -> Result<Delivered, ExecuteError<T>>
    where
        T::Ctx: CtxEntropy,
    {
        if let Some(mark) = self.marks.get(stream)
            && at <= *mark
        {
            return Ok(Delivered::Duplicate);
        }

        let commands = <T as React<F>>::react(target.state(), event, at);
        for command in &commands {
            execute(journal, target, command, ctx)?;
        }
        self.marks.insert(stream.clone(), at);
        Ok(Delivered::Applied)
    }
}
