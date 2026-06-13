#![doc(
    html_logo_url = "https://raw.githubusercontent.com/kassian-dev/ironstate/main/assets/logo.png",
    html_favicon_url = "https://raw.githubusercontent.com/kassian-dev/ironstate/main/assets/favicon-32.png"
)]
#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

// Lets the derive macros emit `::ironstate::…` paths that also resolve inside
// this crate's own tests and doctests.
extern crate self as ironstate;

mod analysis;
mod error;
mod invariant;
mod kind;
mod listener;
mod machine;
#[macro_use]
mod macros;
mod metadata;
mod migrate;
#[cfg(feature = "proptest")]
mod testing;

pub use error::{RestoreError, TransitionError};
pub use invariant::{Invariant, Invariants};
pub use kind::Kind;
pub use listener::TransitionRecord;
pub use machine::{EventKind, Machine, StateMachine, TransitionRules};
pub use metadata::MachineMetadata;
pub use migrate::{MigrateFrom, Versioned};

#[cfg(feature = "derive")]
pub use ironstate_derive::{Event, StateMachine};

/// Graph-analysis report types produced by [`analyze!`].
pub mod analysis_report {
    pub use crate::analysis::{Claim, Confidence, Report, analyze};
}

/// Runtime support invoked by the [`test!`] macro; not part of the stable surface.
#[cfg(feature = "proptest")]
#[doc(hidden)]
pub mod testing_support {
    pub use crate::testing::*;
}

/// Internal runtime invoked by generated derive code; not a stable surface.
#[doc(hidden)]
#[cfg(feature = "restore")]
pub mod __rt {
    pub use crate::migrate::rt::*;
}

/// The common imports for defining and running a machine.
pub mod prelude {
    // `StateMachine` names both the trait and (under `derive`) the derive macro —
    // they live in different namespaces, so a single import brings whichever exist.
    #[cfg(feature = "derive")]
    pub use crate::Event;
    pub use crate::{
        EventKind, Invariant, Invariants, Kind, Machine, MigrateFrom, RestoreError, StateMachine,
        TransitionError, TransitionRules, Versioned,
    };
}
