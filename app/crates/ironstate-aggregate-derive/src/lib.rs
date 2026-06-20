#![doc(
    html_logo_url = "https://raw.githubusercontent.com/kassian-dev/ironstate/main/assets/logo.png",
    html_favicon_url = "https://raw.githubusercontent.com/kassian-dev/ironstate/main/assets/favicon-32.png"
)]
//! Derive macros for `ironstate-aggregate`. You depend on `ironstate-aggregate`,
//! not this crate directly; it re-exports these.
//!
//! - `#[derive(StableHash)]` generates a value's canonical encoding and rejects
//!   field types that cannot be deterministically hashed (floats, hash maps,
//!   wall clocks) with teaching errors.
//! - `#[derive(Redact)]` generates a per-viewer view type and its `view_for`.
//! - `#[derive(Versioned)]` generates versioned restore over a `MigrateFrom`
//!   chain (the `AggregateRules` trait itself is written by hand).

use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

mod redact;
mod stablehash;
mod versioned;

/// Derive `StableHash`: a frozen canonical encoding for determinism digests.
///
/// Fields are encoded in declaration order; enum variants write a
/// declaration-order discriminant first. `#[stable_hash(skip)]` excludes a
/// field (and exempts it from the type scan). Float, `HashMap`/`HashSet`, and
/// `Instant`/`SystemTime` fields are compile errors.
///
/// ```ignore
/// #[derive(StableHash, Clone, Debug, PartialEq)]
/// struct Account {
///     cents: u64,
///     #[stable_hash(skip)] // a derived cache, not part of the state's identity
///     label: String,
/// }
/// ```
#[proc_macro_derive(StableHash, attributes(stable_hash))]
pub fn derive_stable_hash(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    stablehash::derive(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derive `Redact`: a per-principal view type and the `view_for` that builds it.
///
/// Reads `#[redact(principal = P)]` on the struct and, per field, `#[hidden]`
/// (owner sees the value, others a residue), `#[hidden(conceal)]` (everyone sees
/// the residue), or `#[hidden(from = all)]` (omitted from the view). Fields with
/// no attribute appear as-is.
///
/// ```ignore
/// #[derive(Redact, StableHash, Clone, Debug, PartialEq)]
/// #[redact(principal = PlayerId)]
/// struct Match {
///     board: Vec<Card>,                              // public, shown as-is
///     #[hidden] hands: PerPrincipal<PlayerId, Hand>, // owner sees cards, others a count
///     #[hidden(conceal)] deck: Vec<Card>,            // everyone sees only a count
///     #[hidden(from = all)] audit: Digest128,        // in no one's view
/// }
///
/// let what_alice_sees = game.view_for(&alice);
/// ```
#[proc_macro_derive(Redact, attributes(redact, hidden))]
pub fn derive_redact(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    redact::derive(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derive `Versioned`: whole-enum versioning with a `MigrateFrom` chain.
///
/// `history` lists the retired shapes oldest-first; the derive requires a
/// contiguous `MigrateFrom` chain from each to the next, checked at compile time.
/// `restore_versioned` then decodes a `{version, payload}` envelope and walks it
/// forward to the current shape, returning a typed error for a version newer than
/// this binary understands.
///
/// ```ignore
/// use ironstate_aggregate::{MigrateFrom, Versioned};
/// use serde::{Serialize, Deserialize};
///
/// // Retired shapes, kept only so events written by older code still decode.
/// #[derive(Serialize, Deserialize)] enum MatchEventV1 { Joined, Left }
/// #[derive(Serialize, Deserialize)] enum MatchEventV2 { Joined, Left, Renamed }
///
/// #[derive(Versioned, Serialize, Deserialize, Clone, Debug, PartialEq)]
/// #[versioned(version = 3, history = [MatchEventV1, MatchEventV2])]
/// enum MatchEvent { Joined, Left, Renamed, Kicked }
///
/// // One migration per version bump.
/// impl MigrateFrom<MatchEventV1> for MatchEventV2 { /* … */ }
/// impl MigrateFrom<MatchEventV2> for MatchEvent  { /* … */ }
///
/// // A v1 event off the wire decodes, then migrates V1 -> V2 -> current.
/// let current = MatchEvent::restore_versioned(&bytes)?;
/// ```
#[proc_macro_derive(Versioned, attributes(versioned))]
pub fn derive_versioned(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    versioned::derive(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
