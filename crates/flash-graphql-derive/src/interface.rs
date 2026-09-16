//! `#[derive(Interface)]` on an enum whose variants are each `Variant(Inner)`
//! — one per implementing GraphQL object type — matching a typical
//! real-world `Node` interface exactly:
//!
//! ```ignore
//! #[derive(Interface)]
//! #[graphql(field(name = "id", ty = "ID"))]
//! enum Node {
//!     Foo(foo::Foo),
//!     Bar(bar::Bar),
//!     // ...
//! }
//! ```
//!
//! Generated code (a) registers the interface's own declared fields
//! (`field(name = ..., ty = ...)`, container-level, repeatable) via
//! [`flash_graphql::InterfaceType::add_fields`]; (b) records, via
//! `Registrar::implement`, that every variant's inner type must
//! `.implement(this_interface_name)` — the "implements-patching" applied
//! lazily in `Registrar::into_parts` regardless of registration order (see
//! `registrar.rs`); (c) implements `OutputType` for the enum itself so a
//! resolver can return it directly and dynamic dispatch falls out of a plain
//! `match`, producing `FieldValue::{owned,borrowed}_any(inner).with_type(<Inner
//! as OutputType>::type_name())` — the exact shape real dynamic-engine
//! interface resolution requires (`dynamic::resolve::resolve_value`'s
//! `Type::Interface` arm downcasts by that `with_type` name, checked against
//! the interface's `possible_types`, which come from each object's own
//! `implement()` call).

use proc_macro2::TokenStream;
use quote::quote;
use syn::spanned::Spanned;

use crate::util::{
    InterfaceAttrs, MacroError, MacroResult, crate_path, description, parse_graphql_attrs_merged,
    to_pascal_case,
};

pub(crate) fn generate(input: syn::DeriveInput) -> MacroResult<TokenStream> {
    let crate_path = crate_path();
    let ident = input.ident.clone();

    let container_attrs: InterfaceAttrs = parse_graphql_attrs_merged(&input.attrs)?;
    let gql_name = container_attrs
        .name
        .clone()
        .unwrap_or_else(|| to_pascal_case(&ident.to_string()));
    let desc = description(&input.attrs, &None);

    let syn::Data::Enum(data) = &input.data else {
        return Err(
            syn::Error::new(input.span(), "Interface can only be derived for an enum").into(),
        );
    };
    if data.variants.is_empty() {
        return Err(
            syn::Error::new(input.span(), "Interface requires at least one variant").into(),
        );
    }
    if container_attrs.fields.is_empty() {
        return Err(syn::Error::new(
            input.span(),
            "Interface must declare at least one field, e.g. #[graphql(field(name = \"id\", ty = \"ID\"))]",
        )
        .into());
    }

    struct Variant {
        ident: syn::Ident,
        ty: syn::Type,
    }
    let mut variants = Vec::new();
    for v in &data.variants {
        let syn::Fields::Unnamed(fields) = &v.fields else {
            return Err(syn::Error::new(
                v.span(),
                "Interface variants must be of the form `VariantName(InnerType)`",
            )
            .into());
        };
        if fields.unnamed.len() != 1 {
            return Err(syn::Error::new(
                v.span(),
                "Interface variants must wrap exactly one inner type",
            )
            .into());
        }
        variants.push(Variant {
            ident: v.ident.clone(),
            ty: fields.unnamed[0].ty.clone(),
        });
    }

    let field_defs = container_attrs.fields.iter().map(|f| {
        let name = &f.name;
        let ty = &f.ty;
        quote! {
            .field(#crate_path::dynamic::InterfaceField::new(
                #name,
                <#ty as #crate_path::OutputType>::type_ref(),
            ))
        }
    });

    let implement_stmts = variants.iter().map(|v| {
        let ty = &v.ty;
        quote! { registrar.implement::<#ty>(#gql_name); }
    });
    let register_variant_stmts = variants.iter().map(|v| {
        let ty = &v.ty;
        quote! { <#ty as #crate_path::OutputType>::register(registrar); }
    });
    let type_name_arms = variants.iter().map(|v| {
        let v_ident = &v.ident;
        let ty = &v.ty;
        quote! { #ident::#v_ident(_) => <#ty as #crate_path::OutputType>::type_name() }
    });
    let resolve_owned_arms = variants.iter().map(|v| {
        let v_ident = &v.ident;
        quote! { #ident::#v_ident(__inner) => #crate_path::dynamic::FieldValue::owned_any(__inner) }
    });
    let resolve_ref_arms = variants.iter().map(|v| {
        let v_ident = &v.ident;
        quote! { #ident::#v_ident(__inner) => #crate_path::dynamic::FieldValue::borrowed_any(__inner) }
    });

    let interface_desc_call = match &desc {
        Some(d) => quote! { .description(#d) },
        None => quote! {},
    };

    let expanded = quote! {
        impl #crate_path::InterfaceType for #ident {
            const NAME: &'static str = #gql_name;

            fn add_fields(interface: #crate_path::dynamic::Interface) -> #crate_path::dynamic::Interface {
                interface #(#field_defs)*
            }

            fn type_name(&self) -> ::std::borrow::Cow<'static, str> {
                match self {
                    #(#type_name_arms,)*
                }
            }
        }

        impl #crate_path::OutputType for #ident {
            fn type_name() -> ::std::borrow::Cow<'static, str> {
                ::std::borrow::Cow::Borrowed(#gql_name)
            }

            fn register(registrar: &mut #crate_path::Registrar) {
                if !registrar.visit::<Self>() {
                    return;
                }
                #(#implement_stmts)*
                #(#register_variant_stmts)*
                let interface = <Self as #crate_path::InterfaceType>::add_fields(
                    #crate_path::dynamic::Interface::new(#gql_name) #interface_desc_call,
                );
                registrar.register_interface::<Self>(interface);
            }

            fn resolve_owned(self) -> #crate_path::Result<::std::option::Option<#crate_path::dynamic::FieldValue<'static>>> {
                let __type_name = <Self as #crate_path::InterfaceType>::type_name(&self);
                let __value = match self {
                    #(#resolve_owned_arms,)*
                };
                Ok(Some(__value.with_type(__type_name)))
            }

            fn resolve_ref(&self) -> #crate_path::Result<::std::option::Option<#crate_path::dynamic::FieldValue<'_>>> {
                let __type_name = <Self as #crate_path::InterfaceType>::type_name(self);
                let __value = match self {
                    #(#resolve_ref_arms,)*
                };
                Ok(Some(__value.with_type(__type_name)))
            }
        }
    };

    Ok(expanded)
}

pub(crate) fn derive(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    generate(input)
        .unwrap_or_else(MacroError::to_compile_error)
        .into()
}
