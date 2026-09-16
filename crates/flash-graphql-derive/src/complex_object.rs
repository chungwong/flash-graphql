//! `#[ComplexObject] impl Foo { async fn bar(&self, ctx: &Context<'_>, ..) -> X
//! { .. } }` — computed fields added on top of a `#[derive(SimpleObject,
//! ..)] #[graphql(complex)]` struct's plain data fields.
//!
//! Unlike `#[Object]` (see `object.rs`), `Foo` here is a genuine
//! data-carrying type: `SimpleObject`'s `OutputType::resolve_owned`/
//! `resolve_ref` already produce `FieldValue::owned_any(self)`/
//! `borrowed_any(self)`, so every complex-field resolver downcasts
//! `rc.parent_value` back to a real `&Foo` and calls the user's method on it
//! directly — no `Default` requirement, unlike `#[Object]`'s root-only
//! phantom-self types.

use proc_macro2::TokenStream;
use quote::quote;
use syn::spanned::Spanned;

use crate::util::{
    ImplAttrs, MacroError, MacroResult, crate_path, gen_object_field, guard_check, parse_meta_list,
};

pub(crate) fn generate(
    attr_args: TokenStream,
    mut item: syn::ItemImpl,
) -> MacroResult<TokenStream> {
    let crate_path = crate_path();
    let impl_attrs: ImplAttrs = parse_meta_list(attr_args)?;
    let self_ty = item.self_ty.clone();
    let (impl_generics, _, where_clause) = item.generics.split_for_impl();

    let object_guard = guard_check(&impl_attrs.guard, &quote! { rc.ctx })?;
    // `P`/`project` (see `ComplexObjectFields::add_complex_fields`'s doc
    // comment): recovers `&Self` from whatever the resolver's actual parent
    // value is — `Self` itself for a direct registration, or the outer
    // struct when `Self` is `#[graphql(flatten)]`ed into it.
    let self_stmt = quote! {
        let __parent = rc.parent_value.try_downcast_ref::<P>().unwrap();
        let __self_ref = project(__parent);
    };

    let mut add_field_stmts = Vec::new();
    let mut register_stmts = Vec::new();

    for impl_item in &mut item.items {
        let syn::ImplItem::Fn(method) = impl_item else {
            continue;
        };
        if method.sig.asyncness.is_none() {
            return Err(syn::Error::new(
                method.sig.span(),
                "#[ComplexObject] methods must be `async fn`",
            )
            .into());
        }
        let (add_field_stmt, register_stmt) =
            gen_object_field(method, &self_ty, &self_stmt, &object_guard)?;
        add_field_stmts.push(add_field_stmt);
        register_stmts.push(register_stmt);
    }

    let expanded = quote! {
        #item

        impl #impl_generics #crate_path::ComplexObjectFields for #self_ty #where_clause {
            fn add_complex_fields<P: 'static>(
                object: #crate_path::dynamic::Object,
                project: impl ::std::ops::Fn(&P) -> &Self + ::std::marker::Copy + ::std::marker::Send + ::std::marker::Sync + 'static,
            ) -> #crate_path::dynamic::Object {
                #(#add_field_stmts)*
                object
            }

            fn register(registrar: &mut #crate_path::Registrar) {
                #(#register_stmts)*
            }
        }
    };

    Ok(expanded)
}

pub(crate) fn attribute(
    args: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let item = syn::parse_macro_input!(input as syn::ItemImpl);
    generate(args.into(), item)
        .unwrap_or_else(MacroError::to_compile_error)
        .into()
}
