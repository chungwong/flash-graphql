//! `#[derive(Enum)]` — a C-like GraphQL enum. Implements both `OutputType`
//! and `InputType` sharing one `Registrar::visit` slot, matching
//! `tests/smoke.rs`'s hand-written `Color` exactly. Supports struct-level
//! `#[graphql(name = "..")]` and per-variant `#[graphql(name = "..")]`
//! (default: `RenameTarget::EnumItem` = `SCREAMING_SNAKE_CASE`).

use proc_macro2::TokenStream;
use quote::quote;
use syn::spanned::Spanned;

use crate::util::{
    ContainerAttrs, MacroError, MacroResult, VariantAttrs, crate_path, description,
    parse_graphql_attr, to_pascal_case, to_screaming_snake_case,
};

pub(crate) fn generate(input: syn::DeriveInput) -> MacroResult<TokenStream> {
    let crate_path = crate_path();
    let ident = input.ident.clone();

    let container_attrs: ContainerAttrs = parse_graphql_attr(&input.attrs)?;
    let gql_name = container_attrs
        .name
        .clone()
        .unwrap_or_else(|| to_pascal_case(&ident.to_string()));
    // This derive never extracted a container-level `///` doc
    // comment either (unlike `SimpleObject`, which already calls
    // `description(&input.attrs, &None)`) — every `Enum`'s own SDL
    // description was silently dropped (variant descriptions were already
    // handled below, and stayed correct).
    let container_desc = description(&input.attrs, &None);

    let syn::Data::Enum(data) = &input.data else {
        return Err(
            syn::Error::new(input.span(), "Enum can only be derived for a C-like enum").into(),
        );
    };

    struct Variant {
        ident: syn::Ident,
        gql_name: String,
        desc: Option<String>,
    }
    let mut variants = Vec::new();
    for v in &data.variants {
        if !matches!(v.fields, syn::Fields::Unit) {
            return Err(syn::Error::new(v.span(), "Enum variants must not carry data").into());
        }
        let variant_attrs: VariantAttrs = parse_graphql_attr(&v.attrs)?;
        let gql_name = variant_attrs
            .name
            .clone()
            .unwrap_or_else(|| to_screaming_snake_case(&v.ident.to_string()));
        let desc = description(&v.attrs, &variant_attrs.desc);
        variants.push(Variant {
            ident: v.ident.clone(),
            gql_name,
            desc,
        });
    }

    let register_fn = quote::format_ident!("__flash_graphql_register_{}", ident);
    let item_calls = variants.iter().map(|v| {
        let name = &v.gql_name;
        match &v.desc {
            Some(d) => quote! { .item(#crate_path::dynamic::EnumItem::new(#name).description(#d)) },
            None => quote! { .item(#name) },
        }
    });
    let to_name_arms = variants.iter().map(|v| {
        let v_ident = &v.ident;
        let name = &v.gql_name;
        quote! { #ident::#v_ident => #name }
    });
    let from_name_arms = variants.iter().map(|v| {
        let v_ident = &v.ident;
        let name = &v.gql_name;
        quote! { #name => ::std::result::Result::Ok(#ident::#v_ident) }
    });

    let container_desc_call = match &container_desc {
        Some(d) => quote! { .description(#d) },
        None => quote! {},
    };

    let expanded = quote! {
        #[allow(non_snake_case)]
        fn #register_fn(registrar: &mut #crate_path::Registrar) {
            if !registrar.visit::<#ident>() {
                return;
            }
            let e = #crate_path::dynamic::Enum::new(#gql_name)
                #container_desc_call
                #(#item_calls)*;
            registrar.register_enum::<#ident>(e);
        }

        impl #ident {
            fn __flash_graphql_item_name(&self) -> &'static str {
                match self {
                    #(#to_name_arms,)*
                }
            }
        }

        impl #crate_path::OutputType for #ident {
            fn type_name() -> ::std::borrow::Cow<'static, str> {
                ::std::borrow::Cow::Borrowed(#gql_name)
            }

            fn register(registrar: &mut #crate_path::Registrar) {
                #register_fn(registrar);
            }

            fn resolve_owned(self) -> #crate_path::Result<::std::option::Option<#crate_path::dynamic::FieldValue<'static>>> {
                Ok(Some(#crate_path::dynamic::FieldValue::value(#crate_path::Value::Enum(#crate_path::Name::new(self.__flash_graphql_item_name())))))
            }

            fn resolve_ref(&self) -> #crate_path::Result<::std::option::Option<#crate_path::dynamic::FieldValue<'_>>> {
                self.clone().resolve_owned()
            }
        }

        impl #crate_path::InputType for #ident {
            fn type_name() -> ::std::borrow::Cow<'static, str> {
                ::std::borrow::Cow::Borrowed(#gql_name)
            }

            fn register(registrar: &mut #crate_path::Registrar) {
                #register_fn(registrar);
            }

            fn parse(value: ::std::option::Option<#crate_path::Value>) -> #crate_path::Result<Self> {
                let name = match &value {
                    ::std::option::Option::Some(#crate_path::Value::Enum(n)) => n.as_str(),
                    ::std::option::Option::Some(#crate_path::Value::String(s)) => s.as_str(),
                    _ => {
                        return ::std::result::Result::Err(#crate_path::Error::new(::std::format!(
                            "expected enum \"{}\", found {:?}", #gql_name, value,
                        )));
                    }
                };
                match name {
                    #(#from_name_arms,)*
                    other => ::std::result::Result::Err(#crate_path::Error::new(::std::format!(
                        "invalid item for enum \"{}\": \"{}\"", #gql_name, other,
                    ))),
                }
            }

            fn to_value(&self) -> #crate_path::Value {
                #crate_path::Value::Enum(#crate_path::Name::new(self.__flash_graphql_item_name()))
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
