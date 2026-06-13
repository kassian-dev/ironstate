//! Event-kind identifiers used to gate which events a state will accept.

use core::fmt;

/// A compile-time event-kind label.
///
/// Kinds partition events so a state can declare it only accepts certain ones
/// (`#[only_accepts(kind = "external")]`). The inner string is always a
/// `'static` constant produced by the derive, so kinds cost no allocation and
/// can sit directly in the error path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Kind(pub &'static str);

impl Kind {
    /// The label as a string slice.
    pub const fn as_str(&self) -> &'static str {
        self.0
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl PartialEq<str> for Kind {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

/// True if the two kind sets share at least one label.
///
/// An event is accepted by a restricted state when their kind sets intersect.
pub(crate) fn intersects(a: &[Kind], b: &[Kind]) -> bool {
    a.iter().any(|x| b.contains(x))
}
