//! The `#[derive(Versioned)]` implementation for event (and other) enums.
//!
//! Whole-enum versioning with the same `version`/`history` grammar core uses for
//! state machines: the derive requires a contiguous `MigrateFrom` chain at
//! compile time, and generates `restore_versioned`, which decodes a stored
//! payload as the type recorded for its version and migrates it forward.

use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::Parse;
use syn::{DeriveInput, LitInt, Path, Token};

pub fn derive(input: DeriveInput) -> syn::Result<TokenStream> {
    let mut version: Option<u32> = None;
    let mut history: Vec<Path> = Vec::new();
    let mut seen = false;

    for attr in &input.attrs {
        if !attr.path().is_ident("versioned") {
            continue;
        }
        seen = true;
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("version") {
                let lit: LitInt = meta.value()?.parse()?;
                version = Some(lit.base10_parse()?);
            } else if meta.path.is_ident("history") {
                let value = meta.value()?;
                let content;
                syn::bracketed!(content in value);
                history = content
                    .parse_terminated(Path::parse, Token![,])?
                    .into_iter()
                    .collect();
            } else {
                return Err(meta.error("unknown `versioned` option; expected version or history"));
            }
            Ok(())
        })?;
    }

    if !seen {
        return Err(syn::Error::new_spanned(
            &input,
            "missing `#[versioned(...)]` attribute.\n\
             Versioned needs the current version and the retired types.\n\
             Add `#[versioned(version = N, history = [OldV1, …])]`.",
        ));
    }

    let version = version.unwrap_or(1);
    if !history.is_empty() {
        let expected = history.len() as u32 + 1;
        if version != expected {
            return Err(syn::Error::new_spanned(
                &input,
                format!(
                    "version/history mismatch: history lists {} prior version(s), so version \
                     must be {expected}, but it is {version}.\n\
                     `history` is oldest-first and entry i is version i+1; this type is the \
                     last version.\n\
                     Set `version = {expected}` or adjust the history list.",
                    history.len(),
                ),
            ));
        }
    } else if version > 1 {
        return Err(syn::Error::new_spanned(
            &input,
            "version is greater than 1 but no `history` is declared.\n\
             A higher version needs the retired types so restore can migrate forward.\n\
             Add `history = [OldV1, …]` listing every prior version, oldest first.",
        ));
    }

    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let mut arms = vec![quote! {
        #version => ::ironstate::__rt::decode::<Self>(version, payload),
    }];
    for (i, start) in history.iter().enumerate() {
        let v = (i as u32) + 1;
        let mut steps = quote! {
            let value: #start = ::ironstate::__rt::decode(#v, payload)?;
        };
        for next in &history[i + 1..] {
            steps.extend(quote! {
                let value = <#next as ::ironstate::MigrateFrom<_>>::migrate(value);
            });
        }
        steps.extend(quote! {
            ::core::result::Result::Ok(<Self as ::ironstate::MigrateFrom<_>>::migrate(value))
        });
        arms.push(quote! {
            #v => { #steps }
        });
    }

    Ok(quote! {
        impl #impl_generics ::ironstate::Versioned for #name #ty_generics #where_clause {
            const VERSION: u32 = #version;

            fn restore_versioned(bytes: &[u8]) -> ::core::result::Result<Self, ::ironstate::RestoreError> {
                let (version, payload) = ::ironstate::__rt::parse_envelope(bytes)?;
                match version {
                    #(#arms)*
                    v if v > #version => ::core::result::Result::Err(
                        ::ironstate::__rt::newer_than_binary(v, #version)
                    ),
                    v => ::core::result::Result::Err(::ironstate::__rt::unknown_version(v)),
                }
            }
        }
    })
}
