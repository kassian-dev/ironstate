//! Declared invariants — properties that must hold after every transition.

use crate::machine::{EventKind, StateMachine};
use core::marker::PhantomData;

/// The assertion an invariant runs: `(before, event, after) -> holds?`, where
/// `after` is `Some` if the transition committed and `None` if it was rejected.
type Check<S, E> = Box<dyn Fn(&S, &E, &Option<S>) -> bool>;

/// A named property checked after each transition during `test!`.
///
/// The check receives the state before the event, the event, and the state
/// after — `Some` if the transition committed, `None` if it was rejected.
pub struct Invariant<S, E> {
    description: &'static str,
    check: Check<S, E>,
}

impl<S, E> Invariant<S, E> {
    /// Begin defining a custom invariant with a human-readable description.
    pub fn custom(description: &'static str) -> PartialInvariant<S, E> {
        PartialInvariant {
            description,
            _marker: PhantomData,
        }
    }

    /// The invariant's description, shown in failure output.
    pub fn description(&self) -> &'static str {
        self.description
    }

    /// Whether the property holds for this `(before, event, after)` step.
    pub fn holds(&self, before: &S, event: &E, after: &Option<S>) -> bool {
        (self.check)(before, event, after)
    }
}

/// A half-built [`Invariant`] awaiting its assertion closure.
pub struct PartialInvariant<S, E> {
    description: &'static str,
    _marker: PhantomData<fn(&S, &E)>,
}

impl<S, E> PartialInvariant<S, E> {
    /// Supply the property: return `true` when it holds, `false` when violated.
    pub fn assert(self, check: impl Fn(&S, &E, &Option<S>) -> bool + 'static) -> Invariant<S, E> {
        Invariant {
            description: self.description,
            check: Box::new(check),
        }
    }
}

/// Implemented by a machine that declares domain invariants.
///
/// Optional: a machine without an `Invariants` impl is still verified for its
/// structural properties by `test!`. Implement this to add domain-specific
/// checks on top.
pub trait Invariants: StateMachine
where
    Self::Event: EventKind,
{
    /// The invariants to verify after every transition.
    fn invariants() -> Vec<Invariant<Self, Self::Event>>;
}
