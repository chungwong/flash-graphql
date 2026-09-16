//! `#[Object] impl Foo { async fn bar(&self, ctx: &Context<'_>, ..) -> X { .. }
//! }` — a "domain query/mutation split" pattern: `#[Object]`-annotated
//! types are always either a `Schema::build` root directly or a member folded
//! into a `#[derive(MergedObject)]` root (see `merged_object.rs`); either way
//! the outermost schema root's value is discarded by this crate's
//! `Schema::build` (the phantom-marker `RootFields` design — see
//! `schema.rs`'s doc comment: "the marker carries no state"), so there is
//! never a real per-instance `Foo` reaching a resolver here. `#[Object]`
//! methods therefore construct a fresh `Self::default()` per call rather
//! than downcasting a `parent_value` (contrast with `#[ComplexObject]`, whose
//! `Foo` genuinely carries data): matches how `#[Object]` is typically used
//! in practice (zero-sized marker structs, all real data via
//! `ctx.data::<T>()`), and keeps this crate's already-committed
//! `Schema::build` behavior (which never threads a real root value through)
//! intact rather than redesigning it to support a case that doesn't arise.
//!
//! `#[Object]`-derived types directly `impl RootFields` — the same trait
//! `Schema::build` already requires of `Q`/`M`/`S` — rather than a new
//! "`ObjectFields` + projection" trait: since `RootFields::add_fields`/
//! `register` never take `&self` either, `#[derive(MergedObject)]` can just
//! chain member `RootFields::add_fields`/`register` calls directly (see
//! `merged_object.rs`), with no projection machinery needed.

use proc_macro2::TokenStream;
use quote::quote;
use syn::spanned::Spanned;

use crate::util::{
    ImplAttrs, MacroError, MacroResult, crate_path, gen_object_field, guard_check, parse_meta_list,
    to_pascal_case,
};

pub(crate) fn generate(
    attr_args: TokenStream,
    mut item: syn::ItemImpl,
) -> MacroResult<TokenStream> {
    let crate_path = crate_path();
    let impl_attrs: ImplAttrs = parse_meta_list(attr_args)?;
    let self_ty = item.self_ty.clone();
    let self_ty_name = match &*self_ty {
        syn::Type::Path(p) => p
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default(),
        _ => {
            return Err(syn::Error::new(
                self_ty.span(),
                "#[Object] requires a named `impl` self type",
            )
            .into());
        }
    };
    let gql_name = impl_attrs
        .name
        .clone()
        .unwrap_or_else(|| to_pascal_case(&self_ty_name));
    let (impl_generics, _, where_clause) = item.generics.split_for_impl();

    let object_guard = guard_check(&impl_attrs.guard, &quote! { rc.ctx })?;
    let self_stmt = quote! {
        let __self_owned = <#self_ty as ::std::default::Default>::default();
        let __self_ref = &__self_owned;
    };

    let mut add_field_stmts = Vec::new();
    let mut register_stmts = Vec::new();

    for impl_item in &mut item.items {
        let syn::ImplItem::Fn(method) = impl_item else {
            continue;
        };
        if method.sig.asyncness.is_none() {
            return Err(
                syn::Error::new(method.sig.span(), "#[Object] methods must be `async fn`").into(),
            );
        }
        let (add_field_stmt, register_stmt) =
            gen_object_field(method, &self_ty, &self_stmt, &object_guard)?;
        add_field_stmts.push(add_field_stmt);
        register_stmts.push(register_stmt);
    }

    let expanded = quote! {
        #item

        impl #impl_generics #crate_path::RootFields for #self_ty #where_clause {
            const NAME: &'static str = #gql_name;

            fn add_fields(object: #crate_path::dynamic::Object) -> #crate_path::dynamic::Object {
                #(#add_field_stmts)*
                object
            }

            fn register(registrar: &mut #crate_path::Registrar) {
                #(#register_stmts)*
            }
        }

        // Every `#[Object]`-annotated type being a zero-sized marker (this
        // module doc's whole premise) means it can *also* be resolved as a
        // plain nested field's type, not only merged into a schema root —
        // the field's actual (discarded, exactly as `Schema::build` already
        // discards the outermost root's value) instance still resolves to a
        // fresh `Self::default()`-backed object, reusing `RootFields`'s own
        // `add_fields`/`register` rather than duplicating field codegen.
        impl #impl_generics #crate_path::OutputType for #self_ty #where_clause {
            fn type_name() -> ::std::borrow::Cow<'static, str> {
                ::std::borrow::Cow::Borrowed(#gql_name)
            }

            fn register(registrar: &mut #crate_path::Registrar) {
                if !registrar.visit::<Self>() {
                    return;
                }
                <Self as #crate_path::RootFields>::register(registrar);
                let object = #crate_path::dynamic::Object::new(#gql_name);
                let object = <Self as #crate_path::RootFields>::add_fields(object);
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

pub(crate) fn attribute(
    args: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let item = syn::parse_macro_input!(input as syn::ItemImpl);
    generate(args.into(), item)
        .unwrap_or_else(MacroError::to_compile_error)
        .into()
}
