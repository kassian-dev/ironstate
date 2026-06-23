//! The `#[derive(StableHash)]` implementation: canonical-encoding codegen plus
//! the type scans that reject values which cannot be deterministically hashed.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DataEnum, DataStruct, DeriveInput, Field, Fields, Index, Type};

pub fn derive(input: DeriveInput) -> syn::Result<TokenStream> {
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let body = match &input.data {
        Data::Struct(data) => struct_body(data)?,
        Data::Enum(data) => enum_body(data)?,
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                &input,
                "StableHash cannot be derived for unions.\n\
                 A union has no defined active field, so it has no canonical encoding.\n\
                 Use a struct or an enum.",
            ));
        }
    };

    Ok(quote! {
        impl #impl_generics ::ironstate_aggregate::StableHash for #name #ty_generics #where_clause {
            fn encode(&self, enc: &mut ::ironstate_aggregate::CanonicalEncoder) {
                #body
            }
        }
    })
}

fn struct_body(data: &DataStruct) -> syn::Result<TokenStream> {
    let mut encodes = Vec::new();
    for (i, field) in data.fields.iter().enumerate() {
        if is_skip(field) {
            continue;
        }
        check_type(&field.ty)?;
        let accessor = match &field.ident {
            Some(ident) => quote!(#ident),
            None => {
                let index = Index::from(i);
                quote!(#index)
            }
        };
        encodes.push(quote! {
            ::ironstate_aggregate::StableHash::encode(&self.#accessor, enc);
        });
    }
    Ok(quote!( #(#encodes)* ))
}

fn enum_body(data: &DataEnum) -> syn::Result<TokenStream> {
    let mut arms = Vec::new();
    for (variant_index, variant) in data.variants.iter().enumerate() {
        let discriminant = variant_index as u32;
        let vname = &variant.ident;
        let arm = match &variant.fields {
            Fields::Unit => quote! {
                Self::#vname => { enc.write_discriminant(#discriminant); }
            },
            Fields::Unnamed(fields) => {
                let mut binds = Vec::new();
                let mut encodes = Vec::new();
                for (i, field) in fields.unnamed.iter().enumerate() {
                    if is_skip(field) {
                        binds.push(quote!(_));
                    } else {
                        check_type(&field.ty)?;
                        let bind = format_ident!("field_{}", i);
                        encodes.push(quote! {
                            ::ironstate_aggregate::StableHash::encode(#bind, enc);
                        });
                        binds.push(quote!(#bind));
                    }
                }
                quote! {
                    Self::#vname( #(#binds),* ) => {
                        enc.write_discriminant(#discriminant);
                        #(#encodes)*
                    }
                }
            }
            Fields::Named(fields) => {
                let mut pats = Vec::new();
                let mut encodes = Vec::new();
                for field in &fields.named {
                    let ident = field.ident.as_ref().expect("named field");
                    if is_skip(field) {
                        pats.push(quote!(#ident: _));
                    } else {
                        check_type(&field.ty)?;
                        pats.push(quote!(#ident));
                        encodes.push(quote! {
                            ::ironstate_aggregate::StableHash::encode(#ident, enc);
                        });
                    }
                }
                quote! {
                    Self::#vname { #(#pats),* } => {
                        enc.write_discriminant(#discriminant);
                        #(#encodes)*
                    }
                }
            }
        };
        arms.push(arm);
    }
    Ok(quote! {
        match self {
            #(#arms)*
        }
    })
}

/// Whether a field is excluded from the encoding (and from the type scan).
fn is_skip(field: &Field) -> bool {
    let mut skip = false;
    for attr in &field.attrs {
        if !attr.path().is_ident("stable_hash") {
            continue;
        }
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("skip") {
                skip = true;
            }
            Ok(())
        });
    }
    skip
}

/// Reject field types that cannot be deterministically hashed, with a teaching
/// message naming the fix. Recurses through containers so `Vec<f64>` is caught.
///
/// This is a *syntactic* check on the type's last path segment, so it is a
/// teaching aid, not the gate: a type alias (`type Money = f64`) slips past it.
/// The value still cannot be hashed — there is no `StableHash` impl for the
/// forbidden types, so an aliased one fails to compile at its `encode` call with
/// a trait-bound error instead of this message. The type system is the backstop.
fn check_type(ty: &Type) -> syn::Result<()> {
    match ty {
        Type::Path(type_path) => {
            if let Some(segment) = type_path.path.segments.last() {
                if let Some(message) = forbidden(&segment.ident.to_string()) {
                    return Err(syn::Error::new_spanned(ty, message));
                }
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    for arg in &args.args {
                        if let syn::GenericArgument::Type(inner) = arg {
                            check_type(inner)?;
                        }
                    }
                }
            }
            Ok(())
        }
        Type::Reference(r) => check_type(&r.elem),
        Type::Slice(s) => check_type(&s.elem),
        Type::Array(a) => check_type(&a.elem),
        Type::Paren(p) => check_type(&p.elem),
        Type::Group(g) => check_type(&g.elem),
        Type::Tuple(t) => {
            for elem in &t.elems {
                check_type(elem)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn forbidden(ident: &str) -> Option<&'static str> {
    Some(match ident {
        "f32" | "f64" => {
            "floats cannot appear in stable-hashed state.\n\
             Floating-point results are not identical across platforms and optimization \
             levels, so a digest would not be reproducible.\n\
             Use an integer or fixed-point representation, or mark the field \
             `#[stable_hash(skip)]` if it is not part of the value's identity."
        }
        "HashMap" | "HashSet" => {
            "HashMap/HashSet cannot appear in stable-hashed state.\n\
             Their iteration order is randomized, so two equal maps could encode \
             differently.\n\
             Use BTreeMap/BTreeSet (ordered) or PerPrincipal, or mark the field \
             `#[stable_hash(skip)]`."
        }
        "Instant" | "SystemTime" => {
            "wall-clock values cannot be stable-hashed.\n\
             They are not state and not deterministic; time must enter as data.\n\
             Use LogicalTime, or mark the field `#[stable_hash(skip)]`."
        }
        _ => return None,
    })
}
