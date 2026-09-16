//! `#[derive(MergedObject)]` on a tuple struct `struct QueryRoot(TypeA,
//! TypeB, ..)` — flat field concatenation of each member's fields onto one
//! root object.
//!
//! Every member is expected to come from its own `#[Object]` impl, which
//! (see `object.rs`) already `impl RootFields for TypeA { .. }` directly —
//! `add_fields`/`register` never take `&self`, so folding members together
//! is just chaining those associated-function calls in declaration order;
//! no per-member instance/projection is needed (checked against real
//! `async-graphql-derive-7.0.17`'s `merged_object.rs`: there, too, the
//! member *types* never separately appear in the SDL's type registry — only
//! their fields are folded into the merged root's own `MetaType::Object`;
//! matched here by `RootFields::add_fields`/`register` never calling
//! `Registrar::register_object` for a member type, only `Schema::build`
//! calls that, and only for the outermost root).

use proc_macro2::TokenStream;
use quote::quote;
use syn::spanned::Spanned;

use crate::util::{
    ContainerAttrs, MacroError, MacroResult, crate_path, parse_graphql_attr, to_pascal_case,
};

pub(crate) fn generate(input: syn::DeriveInput) -> MacroResult<TokenStream> {
    let crate_path = crate_path();
    let ident = input.ident.clone();
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let container_attrs: ContainerAttrs = parse_graphql_attr(&input.attrs)?;
    let gql_name = container_attrs
        .name
        .clone()
        .unwrap_or_else(|| to_pascal_case(&ident.to_string()));

    let syn::Data::Struct(data) = &input.data else {
        return Err(syn::Error::new(
            input.span(),
            "MergedObject can only be derived for a tuple struct",
        )
        .into());
    };
    let syn::Fields::Unnamed(fields) = &data.fields else {
        return Err(syn::Error::new(
            input.span(),
            "MergedObject requires a tuple struct, e.g. `struct QueryRoot(BranchQuery, SupplierQuery);`",
        )
        .into());
    };
    if fields.unnamed.is_empty() {
        return Err(syn::Error::new(input.span(), "MergedObject needs at least one member").into());
    }

    let member_types: Vec<syn::Type> = fields.unnamed.iter().map(|f| f.ty.clone()).collect();

    let add_fields_chain = member_types.iter().fold(
        quote! { object },
        |acc, ty| quote! { <#ty as #crate_path::RootFields>::add_fields(#acc) },
    );
    let register_calls = member_types
        .iter()
        .map(|ty| quote! { <#ty as #crate_path::RootFields>::register(registrar); });

    let expanded = quote! {
        impl #impl_generics #crate_path::RootFields for #ident #ty_generics #where_clause {
            const NAME: &'static str = #gql_name;

            fn add_fields(object: #crate_path::dynamic::Object) -> #crate_path::dynamic::Object {
                #add_fields_chain
            }

            fn register(registrar: &mut #crate_path::Registrar) {
                #(#register_calls)*
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
