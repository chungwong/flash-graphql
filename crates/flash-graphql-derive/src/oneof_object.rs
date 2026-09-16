//! `#[derive(OneofObject)]` on an enum whose variants are each
//! `Variant(Inner)` — a GraphQL "oneof" input object: from the outside, a set
//! of nullable fields of which exactly one may be supplied; from the inside,
//! a plain Rust enum, one variant per field. Matches a typical real-world
//! usage exactly:
//!
//! ```ignore
//! #[derive(OneofObject)]
//! enum LookupBy {
//!     Id(ID),
//!     Slug(String),
//! }
//! ```
//!
//! **Validation mechanism** (the key design question here): real
//! `async-graphql-7.0.17`'s *dynamic* engine enforces "exactly one field, and
//! it must not be null" itself, at request-validation time, for any
//! `dynamic::InputObject` built with `.oneof()` — see
//! `src/validation/utils.rs`'s `is_valid_value`, which special-cases
//! `MetaType::InputObject { oneof: true, .. }` regardless of whether that
//! `MetaType` came from the dynamic or the static (derive-based) engine. That
//! code is part of the compiled-once, concrete validator (parser/validation
//! are concrete and compile once — nothing to win, nothing to lose either),
//! so this derive
//! only has to call `.oneof()` when building the `dynamic::InputObject` and
//! make every field's `TypeRef` nullable (`dynamic::check.rs` separately
//! enforces that at schema-build time: a `oneof` field must be nullable and
//! have no default value) — it does **not** need to re-implement the
//! "exactly one" check in `parse()`. `parse()` below still mirrors real
//! `async-graphql-derive`'s own defensive `obj.contains_key(field) &&
//! obj.len() == 1` match-and-return-first-hit shape (rather than trusting the
//! invariant blindly), so a `Value` built by hand (bypassing normal request
//! validation — a default value, a test, ...) still gets a clear error
//! instead of an incorrect variant or a panic.

use proc_macro2::TokenStream;
use quote::quote;
use syn::spanned::Spanned;

use crate::util::{
    ContainerAttrs, MacroError, MacroResult, VariantAttrs, crate_path, description,
    parse_graphql_attr, to_camel_case, to_pascal_case,
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
    let desc = description(&input.attrs, &None);

    let syn::Data::Enum(data) = &input.data else {
        return Err(
            syn::Error::new(input.span(), "OneofObject can only be derived for an enum").into(),
        );
    };
    if data.variants.is_empty() {
        return Err(
            syn::Error::new(input.span(), "OneofObject requires at least one variant").into(),
        );
    }

    struct Variant {
        ident: syn::Ident,
        ty: syn::Type,
        gql_name: String,
        desc: Option<String>,
    }
    let mut variants = Vec::new();
    for v in &data.variants {
        let syn::Fields::Unnamed(fields) = &v.fields else {
            return Err(syn::Error::new(
                v.span(),
                "OneofObject variants must be of the form `VariantName(InnerType)` (a single value)",
            )
            .into());
        };
        if fields.unnamed.len() != 1 {
            return Err(syn::Error::new(
                v.span(),
                "OneofObject variants must wrap exactly one value",
            )
            .into());
        }
        let variant_attrs: VariantAttrs = parse_graphql_attr(&v.attrs)?;
        let gql_name = variant_attrs
            .name
            .clone()
            .unwrap_or_else(|| to_camel_case(&v.ident.to_string()));
        let desc = description(&v.attrs, &variant_attrs.desc);
        variants.push(Variant {
            ident: v.ident.clone(),
            ty: fields.unnamed[0].ty.clone(),
            gql_name,
            desc,
        });
    }

    let mut register_stmts = Vec::new();
    let mut field_defs = Vec::new();
    let mut parse_arms = Vec::new();
    let mut to_value_arms = Vec::new();

    for v in &variants {
        let Variant {
            ident: v_ident,
            ty,
            gql_name: field_name,
            desc,
        } = v;
        register_stmts.push(quote! { <#ty as #crate_path::InputType>::register(registrar); });

        let field_desc_call = match desc {
            Some(d) => quote! { .description(#d) },
            None => quote! {},
        };
        field_defs.push(quote! {
            .field(#crate_path::dynamic::InputValue::new(
                #field_name,
                <::std::option::Option<#ty> as #crate_path::InputType>::type_ref(),
            ) #field_desc_call)
        });

        parse_arms.push(quote! {
            if __obj.contains_key(#field_name) && __obj.len() == 1 {
                let __v = __obj.get(#field_name).cloned();
                return ::std::result::Result::Ok(#ident::#v_ident(<#ty as #crate_path::InputType>::parse(__v)?));
            }
        });

        to_value_arms.push(quote! {
            #ident::#v_ident(__v) => {
                __map.insert(#crate_path::Name::new(#field_name), <#ty as #crate_path::InputType>::to_value(__v));
            }
        });
    }

    let object_desc_call = match &desc {
        Some(d) => quote! { .description(#d) },
        None => quote! {},
    };

    let expanded = quote! {
        impl #impl_generics #crate_path::InputType for #ident #ty_generics #where_clause {
            fn type_name() -> ::std::borrow::Cow<'static, str> {
                ::std::borrow::Cow::Borrowed(#gql_name)
            }

            fn register(registrar: &mut #crate_path::Registrar) {
                if !registrar.visit::<Self>() {
                    return;
                }
                #(#register_stmts)*
                let input = #crate_path::dynamic::InputObject::new(#gql_name)
                    #object_desc_call
                    #(#field_defs)*
                    .oneof();
                registrar.register_input_object::<Self>(input);
            }

            fn parse(value: ::std::option::Option<#crate_path::Value>) -> #crate_path::Result<Self> {
                let ::std::option::Option::Some(#crate_path::Value::Object(__obj)) = &value else {
                    return ::std::result::Result::Err(#crate_path::Error::new(::std::format!(
                        "expected oneof input object \"{}\", found {:?}", #gql_name, value,
                    )));
                };
                #(#parse_arms)*
                ::std::result::Result::Err(#crate_path::Error::new(::std::format!(
                    "oneof input object \"{}\" requires exactly one field to be set, found: {:?}",
                    #gql_name,
                    __obj.keys().collect::<::std::vec::Vec<_>>(),
                )))
            }

            fn to_value(&self) -> #crate_path::Value {
                let mut __map = #crate_path::indexmap::IndexMap::new();
                match self {
                    #(#to_value_arms)*
                }
                #crate_path::Value::Object(__map)
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
