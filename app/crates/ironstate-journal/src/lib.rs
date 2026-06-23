#![doc(
    html_logo_url = "https://raw.githubusercontent.com/kassian-dev/ironstate/main/assets/logo.png",
    html_favicon_url = "https://raw.githubusercontent.com/kassian-dev/ironstate/main/assets/favicon-32.png"
)]
//! An event journal for ironstate aggregates — the durable log of what happened.
//!
//! An aggregate is rebuilt by replaying the events that changed it. This crate
//! stores those events and gives you the operations around them:
//!
//! - [`execute`] is the everyday loop — validate a command, append the events it
//!   produces, then apply them — so the log and the in-memory state never
//!   disagree, even if the process crashes mid-step. A store that can't be a
//!   synchronous [`Journal`] (an async one, say) reuses the same discipline via
//!   [`prepare`] + [`Prepared::commit`]/[`Prepared::abort`] around its own append.
//! - [`resume`] rebuilds an aggregate from the log (after a restart, say), and
//!   [`replay`]/[`fork`](Journal::fork) reconstruct or branch its history.
//! - [`replay_hash`] returns a collision-resistant digest of the final state, so
//!   anyone holding the events can verify the outcome.
//!
//! The one subtlety that drives the design: replaying events draws no randomness
//! (only `decide` does), so the random-stream position can't be recomputed from
//! the events — it is recorded **with every append, atomically**. A
//! [`MemoryJournal`] is included as the reference implementation and the yardstick
//! every storage adapter is measured against with [`journal_contract_test!`].
//!
//! # Example: append a command, then replay the log
//!
//! ```
//! use ironstate_journal::{execute, replay, Journal, MemoryJournal, Seq, Snapshot};
//! # use ironstate::prelude::*;
//! # use ironstate_aggregate::*;
//! #
//! # // A tiny aggregate (a tally you can add to) — see the ironstate-aggregate docs.
//! # #[derive(StateMachine, Clone, Debug, PartialEq)]
//! # #[state_machine(initial = Open, terminal = [Closed])]
//! # enum Phase { Open, Closed }
//! # #[derive(Event, Clone, Debug, PartialEq)] enum Step { Close }
//! # impl TransitionRules for Phase {
//! #     type Event = Step;
//! #     fn transition(&self, _: &Step) -> Option<Phase> { matches!(self, Phase::Open).then_some(Phase::Closed) }
//! # }
//! # #[derive(Event, Clone, Debug, PartialEq)] enum Command { Add(u32), Close }
//! # #[derive(Clone, Debug, PartialEq)] enum Change { Added(u32), Closed }
//! # #[derive(Debug)] struct ClosedError;
//! # impl std::fmt::Display for ClosedError {
//! #     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "the counter is closed") }
//! # }
//! # impl std::error::Error for ClosedError {}
//! # #[derive(Clone, Debug, PartialEq)] struct Counter { phase: Phase, total: u32 }
//! # impl AggregateRules for Counter {
//! #     type Phase = Phase; type Command = Command; type Event = Change; type Error = ClosedError;
//! #     type Ctx = OwnedDeterministicCtx<u32>;
//! #     fn phase(&self) -> Phase { self.phase.clone() }
//! #     fn decide(&self, cmd: &Command, _ctx: &mut Self::Ctx) -> Result<Vec<Change>, ClosedError> {
//! #         if self.phase == Phase::Closed { return Err(ClosedError); }
//! #         Ok(match cmd { Command::Add(n) => vec![Change::Added(*n)], Command::Close => vec![Change::Closed] })
//! #     }
//! #     fn evolve(&mut self, change: &Change) {
//! #         match change { Change::Added(n) => self.total += *n, Change::Closed => self.phase = Phase::Closed }
//! #     }
//! # }
//!
//! let genesis = Counter { phase: Phase::Open, total: 0 };
//! let mut journal = MemoryJournal::new(genesis.clone());
//! let mut counter = Aggregate::new(genesis.clone()).unwrap();
//! let mut ctx = OwnedDeterministicCtx {
//!     entropy: Box::new(SeededEntropy::from_seed(&Seed([0; 32]))),
//!     actor: 1,
//!     now: LogicalTime(0),
//! };
//!
//! // The everyday loop: validate, append the events (with the entropy position,
//! // atomically), then apply them — so the log and the live state never disagree.
//! execute(&mut journal, &mut counter, &Command::Add(7), &mut ctx).unwrap();
//! assert_eq!(counter.state().total, 7);
//!
//! // Replaying the log from genesis reproduces the state — the journal's whole point.
//! let events = journal.events_since(None).unwrap();
//! let base = Snapshot { state: genesis, schema_version: 0, at: Seq(0), entropy_pos: DrawPos(0) };
//! assert_eq!(replay(base, &events).unwrap().state().total, 7);
//! ```
#![forbid(unsafe_code)]
#![warn(missing_docs)]

#[cfg(feature = "sim")]
mod contract;
mod journal;
#[macro_use]
mod macros;
#[cfg(feature = "memory")]
mod memory;
mod replay;
#[cfg(feature = "sim")]
mod sim;
mod subscription;

pub use journal::{ExecuteError, Journal, JournalError, Seq, Snapshot, VersionedEvent};
pub use replay::{Prepared, ResumeError, execute, prepare, replay, replay_hash, resume};
pub use subscription::{Delivered, React, StreamId, Subscription};

#[cfg(feature = "memory")]
pub use memory::MemoryJournal;

#[cfg(feature = "sim")]
pub use contract::ContractJournal;

/// The public deterministic-simulation testkit (feature `sim`): reusable fault
/// injection and a reference oracle for consumer DST harnesses.
#[cfg(feature = "sim")]
pub mod testkit {
    pub use crate::sim::{Fault, FaultInjector, FaultSchedule, ReferenceRun};
}

/// Runtime support invoked by the journal test macros; not a stable surface.
#[cfg(feature = "sim")]
#[doc(hidden)]
pub mod testkit_support {
    pub use crate::contract::run_contract;
    pub use crate::sim::run_scenario;
}
