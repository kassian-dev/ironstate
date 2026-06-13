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
#[proc_macro_derive(Redact, attributes(redact, hidden))]
pub fn derive_redact(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    redact::derive(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derive `Versioned`: whole-enum versioning with a `MigrateFrom` chain.
///
/// ```ignore
/// #[derive(Versioned, Clone, Debug)]
/// #[versioned(version = 3, history = [MatchEventV1, MatchEventV2])]
/// enum MatchEvent { /* … */ }
/// ```
#[proc_macro_derive(Versioned, attributes(versioned))]
pub fn derive_versioned(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    versioned::derive(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
