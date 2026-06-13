//! Redaction: what one principal sees of another's state.
//!
//! Two contract traits and two containers. `Conceal` says what residue of an
//! owned value is public; `OwnerRedact` produces a per-viewer view in which the
//! viewer sees their own value in full and everyone else sees only the residue.
//! The generated view types *cannot represent* another principal's hidden value
//! — that exclusion is type-level, not a runtime check.

use crate::rules::{Aggregate, AggregateRules};
use std::collections::BTreeMap;

/// A redacted, per-viewer view of a whole state.
///
/// The `Redact` derive implements this for the state struct, generating an
/// `XView` type. A blanket impl carries it to `Aggregate<X>` too, so a running
/// aggregate can be viewed the same way — done as a trait (not an inherent
/// method) because `Aggregate` is a foreign type to the consumer crate.
pub trait View<P> {
    /// The generated view type.
    type Output;
    /// The view `viewer` is allowed to see of this whole value.
    fn view_for(&self, viewer: &P) -> Self::Output;
}

impl<P, A> View<P> for Aggregate<A>
where
    A: AggregateRules + View<P>,
{
    type Output = <A as View<P>>::Output;
    fn view_for(&self, viewer: &P) -> Self::Output {
        View::view_for(self.state(), viewer)
    }
}

/// What non-owners see of an owned value. Whatever `conceal` returns is, by
/// definition, on the public surface.
pub trait Conceal {
    /// The public residue.
    type Concealed;
    /// Produce the residue.
    fn conceal(&self) -> Self::Concealed;
}

/// Per-viewer redaction for owned or keyed values: the `Redact` derive calls
/// this for every `#[hidden]` field.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be a `#[hidden]` field for principal `{P}`",
    label = "this type does not implement `OwnerRedact<{P}>`",
    note = "a `#[hidden]` field needs per-viewer redaction. The three forms are:\n  \
            - leave it public (no attribute) — every viewer sees the whole field;\n  \
            - `#[hidden(conceal)]` on a `Conceal` type — every viewer sees the residue;\n  \
            - `#[hidden]` on an `OwnerRedact<P>` type (owner full, others the residue) — \
            wrap the value in `PerPrincipal<P, T>` or `Owned<P, T>`."
)]
pub trait OwnerRedact<P> {
    /// The per-viewer view type.
    type View;
    /// The view `viewer` is allowed to see.
    fn redact_for(&self, viewer: &P) -> Self::View;
}

/// A plain, testable derived view of state (scores, overlays). A projection
/// over a *view* is the pattern for per-principal derived UI.
pub trait Projection<A> {
    /// The projected output.
    type Output;
    /// Compute the projection.
    fn project(state: &A) -> Self::Output;
}

// --- PerPrincipal ---------------------------------------------------------

/// A `BTreeMap`-backed per-principal store: ordered, StableHash-clean, and
/// lint-clean by construction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PerPrincipal<P: Ord, T>(BTreeMap<P, T>);

impl<P: Ord, T> PerPrincipal<P, T> {
    /// An empty store.
    pub fn new() -> Self {
        Self(BTreeMap::new())
    }
    /// The value owned by `principal`, if any.
    pub fn get(&self, principal: &P) -> Option<&T> {
        self.0.get(principal)
    }
    /// A mutable reference to `principal`'s value, if any.
    pub fn get_mut(&mut self, principal: &P) -> Option<&mut T> {
        self.0.get_mut(principal)
    }
    /// Insert `value` for `principal`, returning any previous value.
    pub fn insert(&mut self, principal: P, value: T) -> Option<T> {
        self.0.insert(principal, value)
    }
    /// Remove and return `principal`'s value, if any.
    pub fn remove(&mut self, principal: &P) -> Option<T> {
        self.0.remove(principal)
    }
    /// Whether `principal` has an entry.
    pub fn contains_key(&self, principal: &P) -> bool {
        self.0.contains_key(principal)
    }
    /// The number of entries.
    pub fn len(&self) -> usize {
        self.0.len()
    }
    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// Entries in key order.
    pub fn iter(&self) -> impl Iterator<Item = (&P, &T)> {
        self.0.iter()
    }
}

impl<P: Ord, T> Default for PerPrincipal<P, T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<P: Ord + Clone, T: Conceal> Conceal for PerPrincipal<P, T> {
    type Concealed = BTreeMap<P, T::Concealed>;
    fn conceal(&self) -> Self::Concealed {
        self.0
            .iter()
            .map(|(k, v)| (k.clone(), v.conceal()))
            .collect()
    }
}

/// The view of a [`PerPrincipal`] for one viewer: their own value in full, and
/// every other entry only as its residue. It has no way to hold another
/// principal's full value.
pub struct PerPrincipalView<P: Ord, T: Conceal> {
    /// The viewer's own value, if they have an entry.
    pub mine: Option<T>,
    /// Every other entry's public residue, in key order.
    pub others: BTreeMap<P, T::Concealed>,
}

impl<P: Ord + Clone, T: Conceal + Clone> OwnerRedact<P> for PerPrincipal<P, T> {
    type View = PerPrincipalView<P, T>;
    fn redact_for(&self, viewer: &P) -> Self::View {
        let mut mine = None;
        let mut others = BTreeMap::new();
        for (key, value) in &self.0 {
            if key == viewer {
                mine = Some(value.clone());
            } else {
                others.insert(key.clone(), value.conceal());
            }
        }
        PerPrincipalView { mine, others }
    }
}

// The view's derives can't be auto-generated (they would need `T::Concealed`
// bounds the derive won't add), so they are written by hand.
impl<P: Ord + Clone, T: Conceal + Clone> Clone for PerPrincipalView<P, T>
where
    T::Concealed: Clone,
{
    fn clone(&self) -> Self {
        Self {
            mine: self.mine.clone(),
            others: self.others.clone(),
        }
    }
}

impl<P: Ord + core::fmt::Debug, T: Conceal + core::fmt::Debug> core::fmt::Debug
    for PerPrincipalView<P, T>
where
    T::Concealed: core::fmt::Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PerPrincipalView")
            .field("mine", &self.mine)
            .field("others", &self.others)
            .finish()
    }
}

impl<P: Ord, T: Conceal + PartialEq> PartialEq for PerPrincipalView<P, T>
where
    T::Concealed: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.mine == other.mine && self.others == other.others
    }
}

// --- Owned ----------------------------------------------------------------

/// A single value with a known owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owned<P, T> {
    owner: P,
    value: T,
}

impl<P, T> Owned<P, T> {
    /// Wrap `value` as owned by `owner`.
    pub fn new(owner: P, value: T) -> Self {
        Self { owner, value }
    }
    /// The owner.
    pub fn owner(&self) -> &P {
        &self.owner
    }
    /// The owned value.
    pub fn get(&self) -> &T {
        &self.value
    }
    /// A mutable reference to the owned value.
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.value
    }
}

/// The view of an [`Owned`] value: the owner sees it in full, everyone else
/// sees only the residue.
pub enum OwnedView<T: Conceal> {
    /// The viewer is the owner.
    Mine(T),
    /// The viewer is not the owner.
    Concealed(T::Concealed),
}

impl<P: PartialEq, T: Conceal + Clone> OwnerRedact<P> for Owned<P, T> {
    type View = OwnedView<T>;
    fn redact_for(&self, viewer: &P) -> Self::View {
        if &self.owner == viewer {
            OwnedView::Mine(self.value.clone())
        } else {
            OwnedView::Concealed(self.value.conceal())
        }
    }
}

impl<P, T: Conceal> Conceal for Owned<P, T> {
    type Concealed = T::Concealed;
    fn conceal(&self) -> T::Concealed {
        self.value.conceal()
    }
}

impl<T: Conceal + Clone> Clone for OwnedView<T>
where
    T::Concealed: Clone,
{
    fn clone(&self) -> Self {
        match self {
            Self::Mine(t) => Self::Mine(t.clone()),
            Self::Concealed(c) => Self::Concealed(c.clone()),
        }
    }
}

impl<T: Conceal + core::fmt::Debug> core::fmt::Debug for OwnedView<T>
where
    T::Concealed: core::fmt::Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Mine(t) => f.debug_tuple("Mine").field(t).finish(),
            Self::Concealed(c) => f.debug_tuple("Concealed").field(c).finish(),
        }
    }
}

impl<T: Conceal + PartialEq> PartialEq for OwnedView<T>
where
    T::Concealed: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Mine(a), Self::Mine(b)) => a == b,
            (Self::Concealed(a), Self::Concealed(b)) => a == b,
            _ => false,
        }
    }
}

// StableHash for the containers, so a redacted aggregate's state can be hashed.
// Both encode like the BTreeMap/struct they wrap: length-prefixed, key-ordered.
#[cfg(feature = "stablehash")]
impl<P: Ord + crate::StableHash, T: crate::StableHash> crate::StableHash for PerPrincipal<P, T> {
    fn encode(&self, enc: &mut crate::CanonicalEncoder) {
        enc.write_len(self.0.len());
        for (key, value) in &self.0 {
            key.encode(enc);
            value.encode(enc);
        }
    }
}

#[cfg(feature = "stablehash")]
impl<P: crate::StableHash, T: crate::StableHash> crate::StableHash for Owned<P, T> {
    fn encode(&self, enc: &mut crate::CanonicalEncoder) {
        self.owner.encode(enc);
        self.value.encode(enc);
    }
}
