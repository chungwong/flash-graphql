//! `#[Subscription] impl Subscription { #[graphql(guard = "..")] async fn
//! field_name(&self, ctx: &Context<'_>, ..) -> Result<impl Stream<Item = T>>
//! { .. } }` — matches a typical real-world subscription root shape
//! exactly.
//!
//! Same "domain query/mutation split" rationale as `#[Object]` (see
//! `object.rs`'s module doc): a real-world `Subscription` type is typically
//! a zero-sized marker struct (`#[derive(Default)] struct Subscription;`),
//! never itself folded into a `Schema::build` value (the whole schema is one
//! concrete `dynamic::Schema` — see `schema.rs`'s `SubscriptionRoot`
//! doc), so methods here construct a fresh `Self::default()` per call
//! rather than downcasting a `parent_value`, exactly like `#[Object]`.
//!
//! `#[Subscription]`-derived types directly `impl SubscriptionRoot` (the
//! trait the third `Schema::build` slot requires — see
//! `flash-graphql/src/subscription.rs`), the subscription-root counterpart
//! of `#[Object]`'s direct `RootFields` impl.

use proc_macro2::TokenStream;
use quote::quote;
use syn::spanned::Spanned;

use crate::util::{
    ImplAttrs, MacroError, MacroResult, crate_path, gen_subscription_field, guard_check,
    parse_meta_list, to_pascal_case,
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
                "#[Subscription] requires a named `impl` self type",
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
    // Unlike `#[Object]`'s `self_stmt` (`object.rs`) — a plain
    // `let __self_owned = ..; let __self_ref = &__self_owned;` local to the
    // async block — a `#[Subscription]` method's declared return type,
    // `Result<impl Stream<Item = T>>`, is itself an RPIT, and Rust 2024's
    // opaque-type capture rules make it implicitly capture `&self`'s
    // (elided) lifetime whether or not the method body actually borrows
    // `self` (a typical subscription method never does — it only ever
    // touches `ctx`). A same-block stack temporary's borrow can't outlive
    // the block that produces the returned stream, so `object.rs`'s
    // pattern doesn't typecheck here. Since every `#[Subscription]`-tagged
    // type is, like every `#[Object]`-tagged one (see `object.rs`'s
    // module doc), a zero-sized marker struct, `Box::leak`ing a fresh
    // default instance is free (a ZST `Box` never touches the allocator —
    // `Box::new`/`Box::leak` on one just produce a dangling well-aligned
    // pointer, no heap growth) and gives the reference the `'static`
    // lifetime any capture bound could possibly ask for, sidestepping the
    // issue entirely rather than fighting it with `+ use<>` annotations on
    // code this macro doesn't own.
    let self_stmt = quote! {
        let __self_ref: &'static #self_ty = ::std::boxed::Box::leak(::std::boxed::Box::new(
            <#self_ty as ::std::default::Default>::default(),
        ));
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
                "#[Subscription] methods must be `async fn`",
            )
            .into());
        }
        let (add_field_stmt, register_stmt) =
            gen_subscription_field(method, &self_ty, &self_stmt, &object_guard)?;
        add_field_stmts.push(add_field_stmt);
        register_stmts.push(register_stmt);
    }

    let expanded = quote! {
        #item

        impl #impl_generics #crate_path::SubscriptionRoot for #self_ty #where_clause {
            const NAME: &'static str = #gql_name;

            fn add_fields(subscription: #crate_path::dynamic::Subscription) -> #crate_path::dynamic::Subscription {
                #(#add_field_stmts)*
                subscription
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
