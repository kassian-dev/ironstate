//! The `#[derive(StateMachine)]` implementation.

use crate::common::{representative, wildcard_pattern};
use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::Parse;
use syn::{Data, DeriveInput, Ident, LitInt, LitStr, Path, Token, Variant};

struct Config {
    initial: Ident,
    terminal: Vec<Ident>,
    version: Option<u32>,
    history: Vec<Path>,
}

pub fn derive(input: DeriveInput) -> syn::Result<TokenStream> {
    let Data::Enum(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input,
            "StateMachine can only be derived for enums.\n\
             A state machine's states are enum variants.\n\
             Define your states as `enum { … }` and derive StateMachine on it.",
        ));
    };
    let variants: Vec<&Variant> = data.variants.iter().collect();
    let config = parse_config(&input)?;
    validate(&config, &variants)?;

    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let initial_variant = variants
        .iter()
        .find(|v| v.ident == config.initial)
        .expect("validated to exist");
    let initial_expr = representative(initial_variant);

    let terminal_pats = config.terminal.iter().map(|t| {
        let v = variants.iter().find(|v| &v.ident == t).expect("validated");
        wildcard_pattern(v)
    });
    let is_terminal = quote! {
        match self {
            #(#terminal_pats => true,)*
            _ => false,
        }
    };

    let restriction_arms = restriction_arms(&variants);
    let variant_name_arms = variants.iter().map(|v| {
        let pat = wildcard_pattern(v);
        let lit = v.ident.to_string();
        quote!(#pat => #lit)
    });
    let representatives = variants.iter().map(|v| representative(v));

    // Versioned restore is opt-in: only machines that declare `version` or
    // `history` get a `Versioned` impl (and thus the `Deserialize` requirement).
    // Without them, `restore()` behaves exactly as an unversioned machine.
    let versioned = if config.version.is_some() || !config.history.is_empty() {
        let version = config.version.unwrap_or(1);
        versioned_impl(
            name,
            &impl_generics,
            &ty_generics,
            where_clause,
            version,
            &config.history,
        )
    } else {
        TokenStream::new()
    };

    Ok(quote! {
        impl #impl_generics ::ironstate::StateMachine for #name #ty_generics #where_clause {
            fn initial() -> Self {
                #initial_expr
            }

            fn is_terminal(&self) -> bool {
                #is_terminal
            }

            fn restriction(&self) -> ::core::option::Option<&'static [::ironstate::Kind]> {
                #restriction_arms
            }

            fn state_variants() -> ::std::vec::Vec<Self> {
                ::std::vec![ #(#representatives),* ]
            }

            fn variant_name(&self) -> &'static str {
                match self {
                    #(#variant_name_arms,)*
                }
            }
        }

        #versioned
    })
}

fn parse_config(input: &DeriveInput) -> syn::Result<Config> {
    let mut initial: Option<Ident> = None;
    let mut terminal: Vec<Ident> = Vec::new();
    let mut version: Option<u32> = None;
    let mut history: Vec<Path> = Vec::new();
    let mut seen = false;

    for attr in &input.attrs {
        if !attr.path().is_ident("state_machine") {
            continue;
        }
        seen = true;
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("initial") {
                initial = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("terminal") {
                let value = meta.value()?;
                let content;
                syn::bracketed!(content in value);
                let idents = content.parse_terminated(Ident::parse, Token![,])?;
                terminal = idents.into_iter().collect();
            } else if meta.path.is_ident("version") {
                let lit: LitInt = meta.value()?.parse()?;
                version = Some(lit.base10_parse()?);
            } else if meta.path.is_ident("history") {
                let value = meta.value()?;
                let content;
                syn::bracketed!(content in value);
                let paths = content.parse_terminated(Path::parse, Token![,])?;
                history = paths.into_iter().collect();
            } else if meta.path.is_ident("mermaid_docs") {
                // Accepted but unused here; diagram embedding is a separate feature.
            } else {
                return Err(meta.error(
                    "unknown `state_machine` option; expected initial, terminal, version, or history",
                ));
            }
            Ok(())
        })?;
    }

    let initial = initial.ok_or_else(|| {
        syn::Error::new_spanned(
            input,
            "missing `initial` state.\n\
             Every machine needs exactly one initial state.\n\
             Add `#[state_machine(initial = SomeVariant, terminal = [..])]`.",
        )
    })?;
    if !seen {
        return Err(syn::Error::new_spanned(
            input,
            "missing `#[state_machine(...)]` attribute.\n\
             StateMachine needs to know the initial and terminal states.\n\
             Add `#[state_machine(initial = …, terminal = [..])]`.",
        ));
    }
    if terminal.is_empty() {
        return Err(syn::Error::new_spanned(
            input,
            "missing `terminal` states.\n\
             A machine needs at least one terminal state so analysis can check \
             every state can finish.\n\
             Add `terminal = [SomeVariant]` to the `state_machine` attribute.",
        ));
    }

    Ok(Config {
        initial,
        terminal,
        version,
        history,
    })
}

fn validate(config: &Config, variants: &[&Variant]) -> syn::Result<()> {
    let exists = |id: &Ident| variants.iter().any(|v| &v.ident == id);
    if !exists(&config.initial) {
        return Err(syn::Error::new_spanned(
            &config.initial,
            format!("`{}` is not a variant of this enum", config.initial),
        ));
    }
    for t in &config.terminal {
        if !exists(t) {
            return Err(syn::Error::new_spanned(
                t,
                format!("terminal state `{t}` is not a variant of this enum"),
            ));
        }
    }
    if !config.history.is_empty() {
        let expected = config.history.len() as u32 + 1;
        let version = config.version.unwrap_or(1);
        if version != expected {
            return Err(syn::Error::new_spanned(
                &config.initial,
                format!(
                    "version/history mismatch: history lists {} prior version(s), so version \
                     must be {expected}, but it is {version}.\n\
                     `history` is oldest-first and entry i is version i+1; the current type is \
                     the last version.\n\
                     Set `version = {expected}` or adjust the history list.",
                    config.history.len(),
                ),
            ));
        }
    } else if config.version.unwrap_or(1) > 1 {
        return Err(syn::Error::new_spanned(
            &config.initial,
            "version is greater than 1 but no `history` is declared.\n\
             A higher version needs the retired types so restore can migrate forward.\n\
             Add `history = [OldTypeV1, …]` listing every prior version, oldest first.",
        ));
    }
    Ok(())
}

fn restriction_arms(variants: &[&Variant]) -> TokenStream {
    let mut arms = Vec::new();
    for variant in variants {
        if let Some(kinds) = only_accepts_kinds(variant) {
            let pat = wildcard_pattern(variant);
            let kind_lits = kinds.iter().map(|k| quote!(::ironstate::Kind(#k)));
            arms.push(quote! {
                #pat => {
                    const KINDS: &[::ironstate::Kind] = &[ #(#kind_lits),* ];
                    ::core::option::Option::Some(KINDS)
                }
            });
        }
    }
    if arms.is_empty() {
        quote!(::core::option::Option::None)
    } else {
        quote! {
            match self {
                #(#arms,)*
                _ => ::core::option::Option::None,
            }
        }
    }
}

fn only_accepts_kinds(variant: &Variant) -> Option<Vec<String>> {
    for attr in &variant.attrs {
        if !attr.path().is_ident("only_accepts") {
            continue;
        }
        let mut kinds: Vec<String> = Vec::new();
        let parsed = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("kind") {
                let value = meta.value()?;
                if value.peek(syn::token::Bracket) {
                    let content;
                    syn::bracketed!(content in value);
                    let lits = content.parse_terminated(<LitStr as Parse>::parse, Token![,])?;
                    kinds = lits.iter().map(LitStr::value).collect();
                } else {
                    let lit: LitStr = value.parse()?;
                    kinds = vec![lit.value()];
                }
                Ok(())
            } else {
                Err(meta.error("expected `kind = \"…\"` or `kind = [\"…\", \"…\"]`"))
            }
        });
        if parsed.is_ok() && !kinds.is_empty() {
            return Some(kinds);
        }
    }
    None
}

fn versioned_impl(
    name: &Ident,
    impl_generics: &syn::ImplGenerics,
    ty_generics: &syn::TypeGenerics,
    where_clause: Option<&syn::WhereClause>,
    version: u32,
    history: &[Path],
) -> TokenStream {
    // The current-version arm decodes straight into Self.
    let mut arms = vec![quote! {
        #version => ::ironstate::__rt::decode::<Self>(version, payload),
    }];

    // Each historical version decodes into its retired type, then migrates
    // forward through the chain to the current type.
    for (i, start) in history.iter().enumerate() {
        let v = (i as u32) + 1;
        // Types from `start` up to (and including) Self.
        let mut steps = TokenStream::new();
        steps.extend(quote! {
            let value: #start = ::ironstate::__rt::decode(#v, payload)?;
        });
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

    quote! {
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
    }
}
