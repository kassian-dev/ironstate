//! The `#[derive(Event)]` implementation.

use crate::common::{representative, wildcard_pattern};
use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Expr, ExprLit, Lit, Variant};

pub fn derive(input: DeriveInput) -> syn::Result<TokenStream> {
    let Data::Enum(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input,
            "Event can only be derived for enums.\n\
             Events are the inputs that drive a machine's transitions.\n\
             Define them as `enum { … }` and derive Event on it.",
        ));
    };
    let variants: Vec<&Variant> = data.variants.iter().collect();

    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    // kinds(&self)
    let mut kind_arms = Vec::new();
    for variant in &variants {
        if let Some(kinds) = event_kinds(variant)? {
            let pat = wildcard_pattern(variant);
            let lits = kinds.iter().map(|k| quote!(::ironstate::Kind(#k)));
            kind_arms.push(quote! {
                #pat => {
                    const KINDS: &[::ironstate::Kind] = &[ #(#lits),* ];
                    ::core::option::Option::Some(KINDS)
                }
            });
        }
    }
    let kinds_body = if kind_arms.is_empty() {
        quote!(::core::option::Option::None)
    } else {
        quote! {
            match self {
                #(#kind_arms,)*
                _ => ::core::option::Option::None,
            }
        }
    };

    // variant_name(&self)
    let name_arms = variants.iter().map(|v| {
        let pat = wildcard_pattern(v);
        let lit = v.ident.to_string();
        quote!(#pat => #lit)
    });

    // likelihood(&self)
    let mut weight_arms = Vec::new();
    for variant in &variants {
        let weight = likelihood(variant)?;
        let pat = wildcard_pattern(variant);
        weight_arms.push(quote!(#pat => #weight));
    }

    let representatives = variants.iter().map(|v| representative(v));

    Ok(quote! {
        impl #impl_generics ::ironstate::EventKind for #name #ty_generics #where_clause {
            fn kinds(&self) -> ::core::option::Option<&'static [::ironstate::Kind]> {
                #kinds_body
            }

            fn variant_name(&self) -> &'static str {
                match self {
                    #(#name_arms,)*
                }
            }

            fn event_variants() -> ::std::vec::Vec<Self> {
                ::std::vec![ #(#representatives),* ]
            }

            fn likelihood(&self) -> f64 {
                match self {
                    #(#weight_arms,)*
                }
            }
        }
    })
}

fn event_kinds(variant: &Variant) -> syn::Result<Option<Vec<String>>> {
    for attr in &variant.attrs {
        if !attr.path().is_ident("event_kind") {
            continue;
        }
        let nv = attr.meta.require_name_value()?;
        return match &nv.value {
            Expr::Lit(ExprLit {
                lit: Lit::Str(s), ..
            }) => Ok(Some(vec![s.value()])),
            Expr::Array(arr) => {
                let mut kinds = Vec::new();
                for elem in &arr.elems {
                    if let Expr::Lit(ExprLit {
                        lit: Lit::Str(s), ..
                    }) = elem
                    {
                        kinds.push(s.value());
                    } else {
                        return Err(syn::Error::new_spanned(
                            elem,
                            "event_kind list entries must be string literals",
                        ));
                    }
                }
                Ok(Some(kinds))
            }
            other => Err(syn::Error::new_spanned(
                other,
                "event_kind must be a string or a list of strings, e.g. \
                 #[event_kind = \"external\"] or #[event_kind = [\"external\", \"operator\"]]",
            )),
        };
    }
    Ok(None)
}

fn likelihood(variant: &Variant) -> syn::Result<f64> {
    for attr in &variant.attrs {
        if !attr.path().is_ident("likelihood") {
            continue;
        }
        let nv = attr.meta.require_name_value()?;
        return match &nv.value {
            Expr::Lit(ExprLit {
                lit: Lit::Str(s), ..
            }) => label_weight(&s.value(), &nv.value),
            Expr::Lit(ExprLit {
                lit: Lit::Float(f), ..
            }) => f.base10_parse(),
            Expr::Lit(ExprLit {
                lit: Lit::Int(i), ..
            }) => Ok(i.base10_parse::<u32>()? as f64),
            other => Err(syn::Error::new_spanned(
                other,
                "likelihood must be a qualitative label or a number, e.g. \
                 #[likelihood = \"common\"] or #[likelihood = 0.6]",
            )),
        };
    }
    Ok(0.35)
}

fn label_weight(label: &str, span_src: &Expr) -> syn::Result<f64> {
    Ok(match label {
        "almost_always" => 0.85,
        "common" => 0.60,
        "moderate" => 0.35,
        "uncommon" => 0.15,
        "rare" => 0.05,
        "exceptional" => 0.01,
        other => {
            return Err(syn::Error::new_spanned(
                span_src,
                format!(
                    "`{other}` is not a known likelihood label.\n\
                     Expected one of: almost_always, common, moderate, uncommon, rare, \
                     exceptional.\n\
                     Or give a number directly, e.g. #[likelihood = 0.6]."
                ),
            ));
        }
    })
}
