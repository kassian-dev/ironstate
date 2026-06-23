#![doc(
    html_logo_url = "https://raw.githubusercontent.com/kassian-dev/ironstate/main/assets/logo.png",
    html_favicon_url = "https://raw.githubusercontent.com/kassian-dev/ironstate/main/assets/favicon-32.png"
)]
//! Deterministic aggregates: a struct whose state changes through a stream of
//! events, with replay you can trust.
//!
//! An *aggregate* is a consistency boundary — one bank account, one game match —
//! changed by two functions:
//!
//! - [`decide`](AggregateRules::decide) validates a command against the current
//!   state and returns the events that should follow. It is the only place rules
//!   live and the only place randomness is drawn. It does not change state.
//! - [`evolve`](AggregateRules::evolve) applies one event to the state. It is
//!   total and pure: it never fails, never draws randomness, never reads a clock.
//!
//! Because of that split, running the same events again rebuilds the same state
//! byte-for-byte. On top of it the crate adds journaled, seekable entropy (so
//! even randomness replays), redaction ([`#[hidden]`](Redact) fields each viewer
//! sees only their own slice of), and a frozen [`StableHash`] digest that
//! catches any drift. Unlike a core lifecycle machine, an aggregate never
//! listens — its only output is the events it emits.
//!
//! # Quickstart
//!
//! A counter you can bump and close. Note the shape: a [`Phase`](ironstate)
//! machine for the lifecycle, a `Command` (intent, may be rejected), an `Event`
//! (a fact that happened), and the two functions.
//!
//! ```
//! use ironstate::prelude::*;
//! use ironstate_aggregate::*;
//!
//! #[derive(StateMachine, Clone, Debug, PartialEq)]
//! #[state_machine(initial = Open, terminal = [Closed])]
//! enum Phase { Open, Closed }
//!
//! #[derive(Event, Clone, Debug, PartialEq)]
//! enum Step { Close }
//! impl TransitionRules for Phase {
//!     type Event = Step;
//!     fn transition(&self, _: &Step) -> Option<Phase> {
//!         matches!(self, Phase::Open).then_some(Phase::Closed)
//!     }
//! }
//!
//! #[derive(Event, Clone, Debug, PartialEq)]
//! enum Command { Add(u32), Close }
//!
//! #[derive(Clone, Debug, PartialEq)]
//! enum Change { Added(u32), Closed }
//!
//! #[derive(Debug)]
//! struct ClosedError;
//! impl std::fmt::Display for ClosedError {
//!     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//!         write!(f, "the counter is closed")
//!     }
//! }
//! impl std::error::Error for ClosedError {}
//!
//! #[derive(Clone, Debug, PartialEq)]
//! struct Counter { phase: Phase, total: u32 }
//!
//! impl AggregateRules for Counter {
//!     type Phase = Phase;
//!     type Command = Command;
//!     type Event = Change;
//!     type Error = ClosedError;
//!     type Ctx = OwnedDeterministicCtx<u32>;
//!
//!     fn phase(&self) -> Phase { self.phase.clone() }
//!
//!     // The only place rules live; returns the events that should follow.
//!     fn decide(&self, cmd: &Command, _ctx: &mut Self::Ctx) -> Result<Vec<Change>, ClosedError> {
//!         if self.phase == Phase::Closed { return Err(ClosedError); }
//!         Ok(match cmd {
//!             Command::Add(n) => vec![Change::Added(*n)],
//!             Command::Close => vec![Change::Closed],
//!         })
//!     }
//!
//!     // Total and pure: just applies the fact.
//!     fn evolve(&mut self, change: &Change) {
//!         match change {
//!             Change::Added(n) => self.total += *n,
//!             Change::Closed => self.phase = Phase::Closed,
//!         }
//!     }
//! }
//!
//! let mut counter = Aggregate::new(Counter { phase: Phase::Open, total: 0 }).unwrap();
//! let mut ctx = OwnedDeterministicCtx {
//!     entropy: Box::new(SeededEntropy::from_seed(&Seed([0; 32]))),
//!     actor: 1,
//!     now: LogicalTime(0),
//! };
//! counter.handle(&Command::Add(5), &mut ctx).unwrap();
//! counter.handle(&Command::Add(2), &mut ctx).unwrap();
//! assert_eq!(counter.state().total, 7);
//! ```
#![forbid(unsafe_code)]
#![warn(missing_docs)]

// Lets the derive macros emit `::ironstate_aggregate::…` paths that also resolve
// inside this crate's own tests.
extern crate self as ironstate_aggregate;

mod entropy;
#[macro_use]
mod macros;
#[cfg(feature = "redaction")]
mod redaction;
mod rules;
#[cfg(feature = "stablehash")]
mod stablehash;
#[cfg(feature = "proptest")]
mod testkit;

pub use entropy::{
    CtxEntropy, DeterministicCtx, DrawPos, EntropySource, EntropySourceExt, LogicalTime,
    OwnedDeterministicCtx, Seed, SeededEntropy, assert_entropy_contract,
};
pub use rules::{Aggregate, AggregateRules, InitError, Rejection};

// Event/snapshot versioning reuses core's `Versioned`/`MigrateFrom` machinery.
pub use ironstate::{MigrateFrom, RestoreError, Versioned};

#[cfg(feature = "derive")]
pub use ironstate_aggregate_derive::Versioned;

#[cfg(feature = "redaction")]
pub use redaction::{
    Conceal, Owned, OwnedView, OwnerRedact, PerPrincipal, PerPrincipalView, Projection, View,
};

#[cfg(all(feature = "derive", feature = "redaction"))]
pub use ironstate_aggregate_derive::Redact;

/// Runtime support invoked by the test macros; not part of the stable surface.
#[cfg(feature = "proptest")]
#[doc(hidden)]
pub mod testkit_support {
    pub use crate::testkit::*;
}

#[cfg(feature = "proptest")]
pub use testkit::{
    AggregateArbitrary, AggregateInvariant, AggregateInvariants, PartialAggregateInvariant,
};

#[cfg(all(feature = "proptest", feature = "redaction"))]
pub use testkit::LeakTestable;

#[cfg(feature = "stablehash")]
pub use stablehash::{CanonicalEncoder, Digest128, StableHash, digest128};

#[cfg(feature = "audit")]
pub use stablehash::{AuditDigest, audit_digest};

#[cfg(feature = "derive")]
pub use ironstate_aggregate_derive::StableHash;
