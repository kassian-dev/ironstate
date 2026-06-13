//! Shared codegen helpers for the enum derives.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Fields, Variant};

/// A pattern matching this variant while ignoring any data it carries.
///
/// `Self::Unit`, `Self::Tuple(..)`, or `Self::Named { .. }` as appropriate, so
/// callers can match by variant without naming fields.
pub fn wildcard_pattern(variant: &Variant) -> TokenStream {
    let name = &variant.ident;
    match &variant.fields {
        Fields::Unit => quote!(Self::#name),
        Fields::Unnamed(_) => quote!(Self::#name(..)),
        Fields::Named(_) => quote!(Self::#name { .. }),
    }
}

/// An expression constructing one representative value of this variant.
///
/// Data-carrying fields are filled with `Default::default()` — analysis and
/// generation work at the variant level, so the payload only has to exist.
pub fn representative(variant: &Variant) -> TokenStream {
    let name = &variant.ident;
    match &variant.fields {
        Fields::Unit => quote!(Self::#name),
        Fields::Unnamed(fields) => {
            let defaults = fields
                .unnamed
                .iter()
                .map(|_| quote!(::core::default::Default::default()));
            quote!(Self::#name( #(#defaults),* ))
        }
        Fields::Named(fields) => {
            let inits = fields.named.iter().map(|f| {
                let ident = f.ident.as_ref().expect("named field has an identifier");
                quote!(#ident: ::core::default::Default::default())
            });
            quote!(Self::#name { #(#inits),* })
        }
    }
}
