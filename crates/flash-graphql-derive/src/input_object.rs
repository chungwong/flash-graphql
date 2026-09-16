//! `#[derive(InputObject)]` — a GraphQL input object. Supports struct-level
//! `#[graphql(name = "..")]`/`#[graphql(input_name = "..")]` and per-field
//! `#[graphql(name = "..")]`, `#[graphql(skip)]` (excluded from the GraphQL
//! schema; filled via `Default::default()` when constructing `Self`),
//! `#[graphql(default)]`/`#[graphql(default = "expr")]`,
//! `#[graphql(validator(email))]`, `#[graphql(process_with = fn_name)]`, and
//! `#[graphql(secret)]` (accepted, currently inert — see `util.rs`'s
//! `FieldAttrs` doc comment). Matches `tests/smoke.rs`'s hand-written
//! `CreateWidgetInput` exactly.

use proc_macro2::TokenStream;
use quote::quote;
use syn::spanned::Spanned;

use crate::util::{
    ContainerAttrs, FieldAttrs, MacroError, MacroResult, crate_path, default_expr, description,
    parse_graphql_attr, to_camel_case, to_pascal_case,
};

pub(crate) fn generate(input: syn::DeriveInput) -> MacroResult<TokenStream> {
    let crate_path = crate_path();
    let ident = input.ident.clone();
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let container_attrs: ContainerAttrs = parse_graphql_attr(&input.attrs)?;
    // `input_name` wins over `name` — matches real async-graphql-derive's
    // `input_object.rs` exactly (see `util.rs`'s `ContainerAttrs` doc
    // comment).
    let gql_name = container_attrs
        .input_name
        .clone()
        .or_else(|| container_attrs.name.clone())
        .unwrap_or_else(|| to_pascal_case(&ident.to_string()));
    // This derive never extracted `///` doc comments at all
    // (container or field), so every `InputObject`'s SDL description was
    // silently dropped — `ContainerAttrs` has no `desc` field to check
    // against, so `description(&input.attrs, &None)` reads only the doc
    // comment, matching `SimpleObject`'s own container-description call.
    let desc = description(&input.attrs, &None);

    let syn::Data::Struct(data) = &input.data else {
        return Err(
            syn::Error::new(input.span(), "InputObject can only be derived for a struct").into(),
        );
    };
    let syn::Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new(input.span(), "InputObject requires named fields").into());
    };

    struct Field {
        ident: syn::Ident,
        ty: syn::Type,
        gql_name: String,
        desc: Option<String>,
        default: Option<TokenStream>,
        skip: bool,
        validator_email: bool,
        process_with: Option<syn::Expr>,
    }
    let mut all_fields = Vec::new();
    for field in &fields.named {
        let field_attrs: FieldAttrs = parse_graphql_attr(&field.attrs)?;
        let ident = field.ident.clone().expect("named field");
        let gql_name = field_attrs
            .name
            .clone()
            .unwrap_or_else(|| to_camel_case(&ident.to_string()));
        let desc = description(&field.attrs, &field_attrs.desc);
        let default = default_expr(&field_attrs.default)?;
        all_fields.push(Field {
            ident,
            ty: field.ty.clone(),
            gql_name,
            desc,
            default,
            skip: field_attrs.skip,
            validator_email: field_attrs.validator.is_some_and(|v| v.email),
            process_with: field_attrs.process_with.clone(),
        });
    }

    let mut register_stmts = Vec::new();
    let mut field_defs = Vec::new();
    let mut parse_stmts = Vec::new();
    let mut field_idents = Vec::new();
    let mut to_value_stmts = Vec::new();

    for f in &all_fields {
        let Field {
            ident: f_ident,
            ty,
            gql_name,
            desc,
            default,
            skip,
            validator_email,
            process_with,
        } = f;
        field_idents.push(f_ident.clone());
        if *skip {
            parse_stmts.push(quote! { let #f_ident: #ty = ::std::default::Default::default(); });
            continue;
        }
        register_stmts.push(quote! { <#ty as #crate_path::InputType>::register(registrar); });

        let mut input_value = quote! {
            #crate_path::dynamic::InputValue::new(#gql_name, <#ty as #crate_path::InputType>::type_ref())
        };
        if let Some(d) = desc {
            input_value = quote! { #input_value.description(#d) };
        }
        if let Some(d) = default {
            input_value = quote! { #input_value.default_value(<#ty as #crate_path::InputType>::to_value(&(#d))) };
        }
        field_defs.push(quote! { .field(#input_value) });

        let parse_expr = match default {
            Some(d) => quote! {
                match __obj.get(#gql_name).cloned() {
                    ::std::option::Option::Some(__v) => <#ty as #crate_path::InputType>::parse(::std::option::Option::Some(__v))?,
                    ::std::option::Option::None => #d,
                }
            },
            None => quote! {
                <#ty as #crate_path::InputType>::parse(__obj.get(#gql_name).cloned())?
            },
        };
        // `mut` only when `process_with` actually needs to mutate the value
        // in place — kept conditional so a field with neither attr doesn't
        // pick up an `unused_mut` warning.
        let mut_kw = if process_with.is_some() {
            quote! { mut }
        } else {
            quote! {}
        };
        parse_stmts.push(quote! { let #mut_kw #f_ident: #ty = #parse_expr; });
        // `process_with` runs *before* `validator` — matches real
        // async-graphql-derive's own generated argument-parsing order
        // exactly (`derive/src/subscription.rs`'s `get_params`: `#process_with`
        // then `#validators`), so e.g. a field with both
        // `validator(email)` and `process_with = "str_trim_lowercase"`
        // validates the *normalized* value, not the raw one — a
        // leading/trailing-space address gets trimmed first, then checked,
        // not rejected for whitespace the client never meant as part of the
        // address.
        if let Some(pw) = process_with {
            parse_stmts.push(quote! { (#pw)(&mut #f_ident); });
        }
        if *validator_email {
            parse_stmts.push(quote! {
                #crate_path::validators::email(::std::convert::AsRef::<str>::as_ref(&#f_ident))
                    .map_err(|__e| #crate_path::Error::new(::std::format!(
                        "invalid value for \"{}\": {}", #gql_name, __e.message,
                    )))?;
            });
        }

        // Fully-qualified rather than `self.#f_ident.to_value()` — a field
        // type that also implements some *other* trait with an inherent-
        // looking `to_value` method (e.g. sea-orm's `ActiveEnum` trait, when
        // a field is a sea-orm-generated enum type) makes the unqualified
        // method call ambiguous (E0034), even though it never was for a
        // field type with no such collision.
        to_value_stmts.push(quote! {
            __map.insert(#crate_path::Name::new(#gql_name), <#ty as #crate_path::InputType>::to_value(&self.#f_ident));
        });
    }

    let container_desc_call = match &desc {
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
                    #container_desc_call
                    #(#field_defs)*;
                registrar.register_input_object::<Self>(input);
            }

            fn parse(value: ::std::option::Option<#crate_path::Value>) -> #crate_path::Result<Self> {
                let ::std::option::Option::Some(#crate_path::Value::Object(__obj)) = &value else {
                    return ::std::result::Result::Err(#crate_path::Error::new(::std::format!(
                        "expected input object \"{}\", found {:?}", #gql_name, value,
                    )));
                };
                #(#parse_stmts)*
                Ok(#ident {
                    #(#field_idents),*
                })
            }

            fn to_value(&self) -> #crate_path::Value {
                let mut __map = #crate_path::indexmap::IndexMap::new();
                #(#to_value_stmts)*
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
