//! `#[derive(SimpleObject)]` — a plain data-carrying GraphQL object type.
//! Supports struct-level `#[graphql(name = "..")]`/doc-comment description/
//! `#[graphql(complex)]`, and per-field `#[graphql(name = "..")]`,
//! `#[graphql(skip)]`, `#[graphql(desc = "..")]`/doc comments, and
//! `#[graphql(flatten)]`.
//!
//! Every field's own resolver is sync (`FieldFuture::Value`, borrowing
//! through `OutputType::resolve_ref`) — matching `tests/smoke.rs`'s
//! hand-written `Address`/`Widget` exactly, since a plain getter never needs
//! to `.await` anything.

use proc_macro2::TokenStream;
use quote::quote;
use syn::spanned::Spanned;

use crate::util::{
    ContainerAttrs, FieldAttrs, MacroError, MacroResult, crate_path, description, guard_check,
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

    let syn::Data::Struct(data) = &input.data else {
        return Err(syn::Error::new(
            input.span(),
            "SimpleObject can only be derived for a struct",
        )
        .into());
    };
    let syn::Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new(input.span(), "SimpleObject requires named fields").into());
    };

    struct Own {
        ident: syn::Ident,
        ty: syn::Type,
        gql_name: String,
        desc: Option<String>,
        guard: Option<String>,
    }
    enum Member {
        Own(Own),
        Flatten { ty: syn::Type, ident: syn::Ident },
    }

    let mut members = Vec::new();
    for field in &fields.named {
        let field_attrs: FieldAttrs = parse_graphql_attr(&field.attrs)?;
        if field_attrs.skip {
            continue;
        }
        let ident = field.ident.clone().expect("named field");
        if field_attrs.flatten {
            members.push(Member::Flatten {
                ty: field.ty.clone(),
                ident,
            });
            continue;
        }
        let gql_name = field_attrs
            .name
            .clone()
            .unwrap_or_else(|| to_camel_case(&ident.to_string()));
        let desc = description(&field.attrs, &field_attrs.desc);
        members.push(Member::Own(Own {
            ident,
            ty: field.ty.clone(),
            gql_name,
            desc,
            guard: field_attrs.guard.clone(),
        }));
    }

    let mut add_field_stmts = Vec::new();
    let mut register_stmts = Vec::new();
    for member in &members {
        match member {
            Member::Own(Own {
                ident: f_ident,
                ty,
                gql_name,
                desc,
                guard,
            }) => {
                let desc_call = match desc {
                    Some(d) => quote! { .description(#d) },
                    None => quote! {},
                };
                // A plain field's own resolver is sync (`FieldFuture::Value`,
                // see the module doc) — but a `guard = ".."` field needs to
                // `.await` the guard check, so it becomes the same
                // `FieldFuture::Future(Box::pin
                // (async move { .. }))` shape `#[Object]`/`#[ComplexObject]`
                // methods already use (see `util::gen_object_field`), guard
                // first, then the same borrow-and-resolve the sync path does.
                let field_future = match guard_check(guard, &quote! { rc.ctx })? {
                    None => quote! {
                        #crate_path::dynamic::FieldFuture::Value({
                            let __parent = rc.parent_value.try_downcast_ref::<P>().unwrap();
                            let __this = project(__parent);
                            <#ty as #crate_path::OutputType>::resolve_ref(&__this.#f_ident).unwrap()
                        })
                    },
                    Some(guard_stmt) => quote! {
                        #crate_path::dynamic::FieldFuture::Future(::std::boxed::Box::pin(async move {
                            #guard_stmt
                            let __parent = rc.parent_value.try_downcast_ref::<P>().unwrap();
                            let __this = project(__parent);
                            <#ty as #crate_path::OutputType>::resolve_ref(&__this.#f_ident)
                        }))
                    },
                };
                add_field_stmts.push(quote! {
                    let object = object.field(
                        #crate_path::dynamic::Field::new(
                            #gql_name,
                            <#ty as #crate_path::OutputType>::type_ref(),
                            move |rc| { #field_future },
                        )
                        #desc_call,
                    );
                });
                register_stmts.push(quote! {
                    <#ty as #crate_path::OutputType>::register(registrar);
                });
            }
            Member::Flatten { ty, ident: f_ident } => {
                add_field_stmts.push(quote! {
                    let object = <#ty as #crate_path::Flatten>::add_flattened_fields::<P>(object, move |__p: &P| {
                        let __this = project(__p);
                        &__this.#f_ident
                    });
                });
                register_stmts.push(quote! {
                    <#ty as #crate_path::Flatten>::register_flattened(registrar);
                });
            }
        }
    }

    let object_desc_call = match &desc {
        Some(d) => quote! { .description(#d) },
        None => quote! {},
    };

    // These used to run only from the direct (non-flattened)
    // `OutputType::register` path below, so a `#[graphql(complex)]` struct
    // that was itself `#[graphql(flatten)]`ed into another struct silently
    // lost its `#[ComplexObject]` fields from the outer type's SDL (found via
    // a real ORM-entity-backed type that was both `#[graphql(complex)]` and
    // flattened into an outer struct). Folded into
    // `Flatten`'s own two methods instead — using the *same* `P`/`project`
    // `add_flattened_fields` already threads through — so both the direct
    // path (`P = Self`, `project = identity`, called below) and a genuine
    // flatten-into-another-struct both pick them up automatically.
    let complex_add = if container_attrs.complex {
        quote! { let object = <#ident #ty_generics as #crate_path::ComplexObjectFields>::add_complex_fields::<P>(object, project); }
    } else {
        quote! {}
    };
    let complex_register = if container_attrs.complex {
        quote! { <#ident #ty_generics as #crate_path::ComplexObjectFields>::register(registrar); }
    } else {
        quote! {}
    };

    let expanded = quote! {
        impl #impl_generics #crate_path::Flatten for #ident #ty_generics #where_clause {
            fn add_flattened_fields<P: 'static>(
                object: #crate_path::dynamic::Object,
                project: impl ::std::ops::Fn(&P) -> &Self + ::std::marker::Copy + ::std::marker::Send + ::std::marker::Sync + 'static,
            ) -> #crate_path::dynamic::Object {
                #(#add_field_stmts)*
                #complex_add
                object
            }

            fn register_flattened(registrar: &mut #crate_path::Registrar) {
                #(#register_stmts)*
                #complex_register
            }
        }

        impl #impl_generics #crate_path::OutputType for #ident #ty_generics #where_clause {
            fn type_name() -> ::std::borrow::Cow<'static, str> {
                ::std::borrow::Cow::Borrowed(#gql_name)
            }

            fn register(registrar: &mut #crate_path::Registrar) {
                if !registrar.visit::<Self>() {
                    return;
                }
                <Self as #crate_path::Flatten>::register_flattened(registrar);
                let object = #crate_path::dynamic::Object::new(#gql_name) #object_desc_call;
                let object = <Self as #crate_path::Flatten>::add_flattened_fields::<Self>(object, |__x: &Self| __x);
                registrar.register_object::<Self>(object);
            }

            fn resolve_owned(self) -> #crate_path::Result<::std::option::Option<#crate_path::dynamic::FieldValue<'static>>> {
                Ok(Some(#crate_path::dynamic::FieldValue::owned_any(self)))
            }

            fn resolve_ref(&self) -> #crate_path::Result<::std::option::Option<#crate_path::dynamic::FieldValue<'_>>> {
                Ok(Some(#crate_path::dynamic::FieldValue::borrowed_any(self)))
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
