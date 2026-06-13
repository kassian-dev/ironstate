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
//!   disagree, even if the process crashes mid-step.
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
pub use replay::{ResumeError, execute, replay, replay_hash, resume};
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
