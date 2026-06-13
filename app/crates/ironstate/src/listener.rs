//! Observational listeners and the record they receive.
//!
//! Listeners fire *after* a transition has committed or been rejected. They
//! cannot influence the outcome, so they sit outside the sans-I/O law: the
//! transition itself stays a pure function of state and event.

use std::time::Instant;

/// What a transition listener is handed after a successful `apply`.
///
/// The timestamp is observational metadata for audit logs only — it never
/// feeds back into transition logic. It comes from the machine's clock, which
/// is injectable so deterministic harnesses can keep records reproducible.
#[derive(Debug, Clone)]
pub struct TransitionRecord<S, E> {
    /// The state before the transition.
    pub from_state: S,
    /// The event that drove the transition.
    pub event: E,
    /// The state after the transition.
    pub to_state: S,
    /// When the transition was observed.
    pub timestamp: Instant,
}

pub(crate) type TransitionListener<S> =
    Box<dyn Fn(&TransitionRecord<S, <S as crate::machine::TransitionRules>::Event>)>;
pub(crate) type RejectionListener<S> =
    Box<dyn Fn(&crate::error::TransitionError<S, <S as crate::machine::TransitionRules>::Event>)>;
pub(crate) type Clock = Box<dyn Fn() -> Instant>;

pub(crate) fn default_clock() -> Clock {
    Box::new(Instant::now)
}
