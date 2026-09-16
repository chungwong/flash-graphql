//! Shared helpers: attribute parsing, rustdoc-as-description, async-graphql's
//! rename rules, and the argument/guard/default codegen every macro needs.

use darling::FromMeta;
use proc_macro2::TokenStream;
use quote::quote;
use syn::spanned::Spanned;

/// `#[graphql(default)]` / `#[graphql(default = <lit-or-string>)]`. Real
/// async-graphql-derive accepts a bare literal (`default = true`, `default =
/// 90`) *or* a string containing an arbitrary expression (`default =
/// "vec![]"`) side by side — some real-world call sites use the former
/// (`#[graphql(default = true)]`), others the latter, so both need to
/// parse. `darling::util::Override<String>` (used previously) only ever
/// accepted the string form; a bare `true` is a
/// `syn::Lit::Bool`, not a `syn::Lit::Str`, so `String::from_value` rejected
/// it with darling's own "Unexpected type `bool`" error. This type accepts
/// either.
#[derive(Debug, Clone)]
pub(crate) enum DefaultAttr {
    /// Bare `#[graphql(default)]` -> `Default::default()`.
    Inherit,
    /// `#[graphql(default = ..)]`, already-parsed as a Rust expression.
    Expr(TokenStream),
}

impl FromMeta for DefaultAttr {
    fn from_word() -> darling::Result<Self> {
        Ok(DefaultAttr::Inherit)
    }

    fn from_value(value: &syn::Lit) -> darling::Result<Self> {
        let ts = match value {
            syn::Lit::Str(s) => {
                let expr: syn::Expr = syn::parse_str(&s.value())
                    .map_err(|e| darling::Error::custom(e.to_string()).with_span(s))?;
                quote! { #expr }
            }
            syn::Lit::Bool(b) => quote! { #b },
            syn::Lit::Int(i) => quote! { #i },
            syn::Lit::Float(f) => quote! { #f },
            syn::Lit::Char(c) => quote! { #c },
            other => return Err(darling::Error::unexpected_lit_type(other)),
        };
        Ok(DefaultAttr::Expr(ts))
    }
}

/// A single unified error type so every generator fn can just use `?`
/// against both `syn::Error` and `darling::Error`, mirroring
/// `async-graphql-derive`'s own `GeneratorError`.
pub(crate) enum MacroError {
    Syn(syn::Error),
    Darling(darling::Error),
}

impl From<syn::Error> for MacroError {
    fn from(e: syn::Error) -> Self {
        MacroError::Syn(e)
    }
}

impl From<darling::Error> for MacroError {
    fn from(e: darling::Error) -> Self {
        MacroError::Darling(e)
    }
}

impl MacroError {
    #[allow(clippy::wrong_self_convention)] // one-shot consuming conversion, not a `to_*` accessor
    pub(crate) fn to_compile_error(self) -> TokenStream {
        match self {
            MacroError::Syn(e) => e.to_compile_error(),
            MacroError::Darling(e) => e.write_errors(),
        }
    }
}

pub(crate) type MacroResult<T> = Result<T, MacroError>;

/// `::flash_graphql`, the absolute path macro-generated code always uses —
/// this derive crate is only ever consumed through `flash-graphql`'s facade
/// (`pub use flash_graphql_derive::{..}`), so the end-user crate that
/// actually expands these macros always has `flash-graphql` (not
/// `flash-graphql-derive`) as a direct dependency, and `::flash_graphql::..`
/// resolves there.
pub(crate) fn crate_path() -> TokenStream {
    quote! { ::flash_graphql }
}

// ---------------------------------------------------------------------------
// Attribute parsing
// ---------------------------------------------------------------------------

/// `#[graphql(name = "..", complex, input_name = "..")]` on a
/// `SimpleObject`/`InputObject`/`Enum` struct or enum. `input_name`
/// (deferred from an earlier pass, now added) is only ever read by
/// `InputObject`'s own name resolution (`input_name.or(name)` — matches
/// real async-graphql-derive's `input_object.rs`: `object_args.input_name.clone().or_else(|| object_args
/// .name.clone())`, checked directly against `async-graphql-derive-7.0.17`);
/// `SimpleObject`/`Enum` simply never read it. A real-world use case: a
/// filter-input module with several `#[graphql(input_name = "..")]`
/// structs, each giving a Rust type a different (real, unprefixed) GraphQL
/// input type name.
#[derive(Debug, Default, FromMeta)]
#[darling(default)]
pub(crate) struct ContainerAttrs {
    pub name: Option<String>,
    pub complex: bool,
    pub input_name: Option<String>,
}

/// `#[graphql(validator(email))]` — currently only the bare `email` check
/// is implemented (the only one a real-world port actually needed); real
/// async-graphql-derive's `Validators` supports a much larger set
/// (`min_length`, `regex`, custom validator paths, ...) this crate has no
/// need to reimplement yet.
#[derive(Debug, Default, FromMeta)]
#[darling(default)]
pub(crate) struct ValidatorSpec {
    pub email: bool,
}

/// `#[graphql(name = "..", desc = "..", skip, flatten, default, validator(..),
/// process_with = .., secret)]` on a `SimpleObject`/`InputObject` field.
///
/// `validator`/`process_with`/`secret` are deferred-then-added features:
/// - `validator(email)`: after parsing, validate the string is an
///   email-shaped value (`flash_graphql::validators::email`, same
///   `fast_chemail`-backed check real async-graphql's own
///   `validators::email` uses — see that crate's `src/validators/email.rs`
///   — reimplemented here rather than called directly, since the real one
///   is bounded by `T: async_graphql::InputType`, the heavy trait this
///   design exists to avoid on user types), returning a clean parse-time
///   error rather than silently accepting a malformed address.
/// - `process_with = fn_name` (bare path or a string literal spelling one —
///   darling's `syn::Expr: FromMeta` impl already accepts both forms, seen
///   in real-world usage as both `process_with = trim_username` and
///   `process_with = "str_trim_lowercase"` side by side): calls
///   `fn_name(&mut self.field)` after parsing, before the field is used.
/// - `secret`: accepted but currently inert — real async-graphql's `secret`
///   only redacts a value when re-stringifying an *executed query document*
///   for tracing/logging (`registry::stringify_exec_doc`, an extension this
///   crate doesn't implement), not SDL visibility or parsing; a real-world
///   usage on a password-hash-style field always pairs it with `skip`,
///   which already fully removes the field, so accepting the attribute
///   (rather than failing to parse it) is the only thing that usage needs.
/// - `guard = ".."`: only meaningful on a `SimpleObject` field (see
///   `simple_object.rs`'s own codegen) — `InputObject`'s derive never reads
///   it, so it is silently inert there, same as every other field a given
///   derive doesn't use.
#[derive(Debug, Default, FromMeta)]
#[darling(default)]
pub(crate) struct FieldAttrs {
    pub name: Option<String>,
    pub desc: Option<String>,
    pub skip: bool,
    pub flatten: bool,
    pub default: Option<DefaultAttr>,
    pub guard: Option<String>,
    pub validator: Option<ValidatorSpec>,
    pub process_with: Option<syn::Expr>,
    pub secret: bool,
}

/// `#[graphql(name = "..", desc = "..")]` on an `Enum` variant, and (reused,
/// same shape) on an `OneofObject` variant.
#[derive(Debug, Default, FromMeta)]
#[darling(default)]
pub(crate) struct VariantAttrs {
    pub name: Option<String>,
    pub desc: Option<String>,
}

/// One `#[graphql(field(name = "..", ty = ".."))]` entry at the top of a
/// `#[derive(Interface)]` enum — an interface-level field declaration. `ty`
/// parses as a real Rust type (e.g. `ID`, must be in scope where the enum is
/// declared), matching real async-graphql-derive's `InterfaceField::ty:
/// syn::Type` exactly (so the interface field's `TypeRef` comes from that
/// type's own `OutputType::type_ref()`, same as everywhere else in this
/// crate, rather than a hand-parsed GraphQL type string).
#[derive(Debug, FromMeta)]
pub(crate) struct InterfaceFieldSpec {
    pub name: String,
    pub ty: syn::Type,
}

/// `#[graphql(name = "..", field(..), field(..), ..)]` container attrs for
/// `#[derive(Interface)]`. `fields` is `multiple` — collected across every
/// `#[graphql(..)]` attribute on the item, not just the first, via
/// [`parse_graphql_attrs_merged`] (unlike every other derive here, which only
/// ever needs the first `#[graphql(..)]` attribute).
#[derive(Debug, Default, FromMeta)]
#[darling(default)]
pub(crate) struct InterfaceAttrs {
    pub name: Option<String>,
    #[darling(multiple, rename = "field")]
    pub fields: Vec<InterfaceFieldSpec>,
}

/// Like [`parse_graphql_attr`], but merges the nested meta lists of *every*
/// `#[graphql(..)]` attribute on the item into one before parsing as `T` —
/// needed for `#[derive(Interface)]`'s repeatable `field(..)` entries, which
/// real-world code may (and does, in general) spread across more than one
/// `#[graphql(..)]` attribute rather than one attribute with several
/// comma-separated `field(..)` items.
pub(crate) fn parse_graphql_attrs_merged<T: FromMeta + Default>(
    attrs: &[syn::Attribute],
) -> MacroResult<T> {
    let mut nested = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("graphql")
            && let syn::Meta::List(list) = &attr.meta
        {
            nested.extend(darling::ast::NestedMeta::parse_meta_list(
                list.tokens.clone(),
            )?);
        }
    }
    if nested.is_empty() {
        return Ok(T::default());
    }
    Ok(T::from_list(&nested)?)
}

/// `#[Object(name = "..", guard = "..")]` / `#[ComplexObject(..)]` — the
/// attribute macro's own args (a bare meta list, not wrapped in
/// `#[graphql(..)]`).
#[derive(Debug, Default, FromMeta)]
#[darling(default)]
pub(crate) struct ImplAttrs {
    pub name: Option<String>,
    pub guard: Option<String>,
}

/// `#[graphql(name = "..", guard = "..", desc = "..")]` on an `#[Object]` /
/// `#[ComplexObject]` method.
#[derive(Debug, Default, FromMeta)]
#[darling(default)]
pub(crate) struct MethodAttrs {
    pub name: Option<String>,
    pub guard: Option<String>,
    pub desc: Option<String>,
}

/// `#[graphql(name = "..", desc = "..", default = ..)]` on a method argument.
#[derive(Debug, Default, FromMeta)]
#[darling(default)]
pub(crate) struct ArgAttrs {
    pub name: Option<String>,
    pub desc: Option<String>,
    pub default: Option<DefaultAttr>,
}

/// Find the (at most one) `#[graphql(..)]` attribute among `attrs` and parse
/// it as `T`; `T::default()` if there is none.
pub(crate) fn parse_graphql_attr<T: FromMeta + Default>(
    attrs: &[syn::Attribute],
) -> MacroResult<T> {
    for attr in attrs {
        if attr.path().is_ident("graphql") {
            return Ok(T::from_meta(&attr.meta)?);
        }
    }
    Ok(T::default())
}

/// Strip `#[graphql(..)]` from `attrs` — needed wherever we re-emit an
/// original item (a field, a method signature, an argument pattern) that
/// carried one, since `graphql` is not a real attribute macro on its own.
pub(crate) fn remove_graphql_attrs(attrs: &mut Vec<syn::Attribute>) {
    attrs.retain(|a| !a.path().is_ident("graphql"));
}

/// Parse `#[Object(..)]`/`#[ComplexObject(..)]`'s own attribute-macro args
/// (a bare `NestedMeta` list, e.g. `name = "Query", guard = ".."`) as `T`.
pub(crate) fn parse_meta_list<T: FromMeta + Default>(
    args: proc_macro2::TokenStream,
) -> MacroResult<T> {
    if args.is_empty() {
        return Ok(T::default());
    }
    let nested = darling::ast::NestedMeta::parse_meta_list(args)?;
    Ok(T::from_list(&nested)?)
}

/// Doc-comment (`///`) text as a GraphQL description: each `#[doc = ".."]`
/// line trimmed, joined with `\n`, whole thing trimmed. `None` if there are
/// no doc comments at all.
pub(crate) fn doc_comment(attrs: &[syn::Attribute]) -> Option<String> {
    let mut lines = Vec::new();
    for attr in attrs {
        if let syn::Meta::NameValue(nv) = &attr.meta
            && nv.path.is_ident("doc")
            && let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(s),
                ..
            }) = &nv.value
        {
            lines.push(s.value());
        }
    }
    if lines.is_empty() {
        return None;
    }
    let joined = lines
        .iter()
        .map(|l| l.trim())
        .collect::<Vec<_>>()
        .join("\n");
    let trimmed = joined.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// `desc` attr wins over a doc comment; `None` if neither is present.
pub(crate) fn description(attrs: &[syn::Attribute], desc_attr: &Option<String>) -> Option<String> {
    desc_attr.clone().or_else(|| doc_comment(attrs))
}

// ---------------------------------------------------------------------------
// async-graphql's rename rules (`derive/src/args.rs::RenameRule`/`RenameTarget`)
// ---------------------------------------------------------------------------

/// snake_case (or already-PascalCase) -> PascalCase. Default rename for a
/// GraphQL *type* name. Only the first character of each `_`-separated
/// segment is touched (capitalized) — the rest of the segment is left
/// exactly as written, so this is idempotent on an already-PascalCase Rust
/// type name (the overwhelmingly common case for a struct/enum ident)
/// instead of destroying its internal casing (`QueryRoot` must stay
/// `QueryRoot`, not become `Queryroot`).
pub(crate) fn to_pascal_case(s: &str) -> String {
    // A raw identifier (`r#in`, `r#type`, ... — a Rust keyword
    // used as a field/arg name, e.g. an `in: Option<Vec<i32>>` filter field)
    // round-trips through
    // `syn::Ident::to_string()` *with* the `r#` prefix still attached, so
    // the default GraphQL name came out `rIn`/`R_IN` instead of `in`/`IN`.
    // Stripping it here (the one place every other case-conversion call
    // funnels through — `to_camel_case` calls this) covers every call site
    // at once.
    let s = s.strip_prefix("r#").unwrap_or(s);
    // Splitting on `_` alone under-segments a word like
    // `printed_under_1d` — `1d` stayed one word, capitalizing to `1d`
    // (a no-op on the leading digit), so `printedUnder1d` came out instead
    // of real async-graphql-derive's (Inflector's) `printedUnder1D`. A digit
    // run is its own word exactly like an `_`-delimited one, in both
    // directions (`1d` -> `1`, `d`; trailing-digit words like `month1` are
    // unaffected either way, since capitalizing a digit is a no-op).
    let mut words = Vec::new();
    let mut current = String::new();
    let mut prev_is_digit = None;
    for c in s.chars() {
        if c == '_' {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            prev_is_digit = None;
            continue;
        }
        let is_digit = c.is_ascii_digit();
        if prev_is_digit.is_some_and(|prev: bool| prev != is_digit) {
            words.push(std::mem::take(&mut current));
        }
        current.push(c);
        prev_is_digit = Some(is_digit);
    }
    if !current.is_empty() {
        words.push(current);
    }

    words
        .into_iter()
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

/// snake_case -> camelCase. Default rename for a GraphQL *field*/*argument*
/// name.
pub(crate) fn to_camel_case(s: &str) -> String {
    let pascal = to_pascal_case(s);
    let mut chars = pascal.chars();
    match chars.next() {
        Some(c) => c.to_lowercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// PascalCase (an enum variant ident) -> SCREAMING_SNAKE_CASE. Default
/// rename for a GraphQL *enum item* name.
pub(crate) fn to_screaming_snake_case(s: &str) -> String {
    let s = s.strip_prefix("r#").unwrap_or(s);
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c == '_' {
            out.push('_');
            continue;
        }
        if i > 0 && c.is_uppercase() {
            let prev = chars[i - 1];
            let next_is_lower = chars.get(i + 1).is_some_and(|c| c.is_lowercase());
            if prev.is_lowercase()
                || prev.is_ascii_digit()
                || (prev.is_uppercase() && next_is_lower)
            {
                out.push('_');
            }
        } else if i > 0 && c.is_ascii_digit() {
            // A letter-to-digit transition needs a boundary too (e.g.
            // `From0To1Day` -> `FROM_0_TO_1_DAY`, not `FROM0_TO1_DAY` — the
            // old rule only ever inserted `_` before an *uppercase* letter,
            // never before a digit run starting).
            let prev = chars[i - 1];
            if !prev.is_ascii_digit() {
                out.push('_');
            }
        }
        out.push(c);
    }
    out.to_uppercase()
}

// ---------------------------------------------------------------------------
// Shared codegen: a Rust expression from a `default`/`guard` attribute value
// ---------------------------------------------------------------------------

/// `#[graphql(default)]` (bare) -> `Default::default()`; `#[graphql(default =
/// true)]`/`#[graphql(default = "expr")]` -> the literal or (parsed as a Rust
/// expression) string, spliced in verbatim.
pub(crate) fn default_expr(default: &Option<DefaultAttr>) -> MacroResult<Option<TokenStream>> {
    match default {
        None => Ok(None),
        Some(DefaultAttr::Inherit) => Ok(Some(quote! { ::std::default::Default::default() })),
        Some(DefaultAttr::Expr(ts)) => Ok(Some(quote! { (#ts) })),
    }
}

/// `#[graphql(guard = "expr")]` -> a statement that checks the guard (with
/// `GuardExt` in scope, so `.and`/`.or` work) and bails with `?` on failure.
/// `ctx_expr` is the in-scope `&Context<'_>` token stream to check against
/// (`rc.ctx` in every call site in this crate).
pub(crate) fn guard_check(
    guard: &Option<String>,
    ctx_expr: &TokenStream,
) -> MacroResult<Option<TokenStream>> {
    let crate_path = crate_path();
    match guard {
        None => Ok(None),
        Some(s) => {
            let expr: syn::Expr = syn::parse_str(s)?;
            Ok(Some(quote! {
                #crate_path::Guard::check(&{
                    #[allow(unused_imports)]
                    use #crate_path::GuardExt as _;
                    #expr
                }, #ctx_expr).await?;
            }))
        }
    }
}

// ---------------------------------------------------------------------------
// Method argument extraction, shared by `#[Object]` and `#[ComplexObject]`
// ---------------------------------------------------------------------------

pub(crate) struct MethodArg {
    pub ident: syn::Ident,
    pub ty: syn::Type,
    pub gql_name: String,
    pub desc: Option<String>,
    pub default: Option<DefaultAttr>,
}

pub(crate) struct ParsedMethod {
    /// The identifier the user bound `&Context<'_>` to (if the method takes
    /// one at all) — e.g. `ctx` in `async fn widgets(&self, ctx: &Context<'_>, ..)`.
    pub ctx_ident: Option<syn::Ident>,
    pub args: Vec<MethodArg>,
}

fn is_context_type(ty: &syn::Type) -> bool {
    if let syn::Type::Reference(r) = ty
        && let syn::Type::Path(p) = &*r.elem
    {
        return p
            .path
            .segments
            .last()
            .map(|s| s.ident == "Context")
            .unwrap_or(false);
    }
    false
}

/// Walk a method signature's non-`self` parameters, pulling out the
/// `&Context<'_>` parameter (if any) and every GraphQL argument (parsing and
/// then stripping its `#[graphql(..)]` attr in place).
pub(crate) fn parse_method_args(sig: &mut syn::Signature) -> MacroResult<ParsedMethod> {
    let mut ctx_ident = None;
    let mut args = Vec::new();
    for input in sig.inputs.iter_mut() {
        let pat_type = match input {
            syn::FnArg::Receiver(_) => continue,
            syn::FnArg::Typed(pat_type) => pat_type,
        };
        if is_context_type(&pat_type.ty) {
            if let syn::Pat::Ident(pi) = &*pat_type.pat {
                ctx_ident = Some(pi.ident.clone());
            }
            continue;
        }
        let attrs: ArgAttrs = parse_graphql_attr(&pat_type.attrs)?;
        remove_graphql_attrs(&mut pat_type.attrs);
        let ident = match &*pat_type.pat {
            syn::Pat::Ident(pi) => pi.ident.clone(),
            other => {
                return Err(syn::Error::new(
                    other.span(),
                    "flash-graphql: expected a simple argument name",
                )
                .into());
            }
        };
        let gql_name = attrs
            .name
            .clone()
            .unwrap_or_else(|| to_camel_case(&ident.to_string()));
        args.push(MethodArg {
            ident,
            ty: (*pat_type.ty).clone(),
            gql_name,
            desc: attrs.desc.clone(),
            default: attrs.default.clone(),
        });
    }
    Ok(ParsedMethod { ctx_ident, args })
}

/// For one [`MethodArg`], generate (a) the `let name: Ty = ..;` binding read
/// from `rc.args`, applying the default when absent, and (b) the
/// `.argument(InputValue::new(..))` builder call to chain onto the `Field`.
pub(crate) fn gen_arg(arg: &MethodArg) -> MacroResult<(TokenStream, TokenStream)> {
    let crate_path = crate_path();
    let MethodArg {
        ident,
        ty,
        gql_name,
        desc,
        default,
    } = arg;
    let default_ts = default_expr(default)?;

    let get = quote! { rc.args.get(#gql_name).map(|v| v.as_value().clone()) };
    let binding = match &default_ts {
        Some(d) => quote! {
            let #ident: #ty = match #get {
                ::std::option::Option::Some(__v) => <#ty as #crate_path::InputType>::parse(::std::option::Option::Some(__v))?,
                ::std::option::Option::None => #d,
            };
        },
        None => quote! {
            let #ident: #ty = <#ty as #crate_path::InputType>::parse(#get)?;
        },
    };

    let mut input_value = quote! {
        #crate_path::dynamic::InputValue::new(#gql_name, <#ty as #crate_path::InputType>::type_ref())
    };
    if let Some(d) = desc {
        input_value = quote! { #input_value.description(#d) };
    }
    if let Some(d) = &default_ts {
        input_value = quote! {
            #input_value.default_value(<#ty as #crate_path::InputType>::to_value(&(#d)))
        };
    }
    let arg_call = quote! { .argument(#input_value) };
    Ok((binding, arg_call))
}

// ---------------------------------------------------------------------------
// Shared `#[Object]`/`#[ComplexObject]` method -> `Field` codegen
// ---------------------------------------------------------------------------

/// Turn one `async fn` method into (a) a `let object = object.field(..);`
/// statement to fold into `add_fields`/`add_complex_fields`, and (b) the
/// matching `register` statement for its return/argument types. Strips the
/// method's own `#[graphql(..)]` attr in place (the method itself is
/// re-emitted unchanged alongside the generated impl).
///
/// `self_stmt` must bind a local named `__self_ref: &#self_ty` — the two
/// callers differ only in how they produce it: `#[Object]` methods are
/// root-only (no real per-instance data ever reaches `Schema::build`, see
/// `schema.rs`'s `RootFields`), so they construct a fresh `Self::default()`;
/// `#[ComplexObject]` methods downcast the real `parent_value` instead.
pub(crate) fn gen_object_field(
    method: &mut syn::ImplItemFn,
    self_ty: &syn::Type,
    self_stmt: &TokenStream,
    object_guard: &Option<TokenStream>,
) -> MacroResult<(TokenStream, TokenStream)> {
    let crate_path = crate_path();
    let method_attrs: MethodAttrs = parse_graphql_attr(&method.attrs)?;
    remove_graphql_attrs(&mut method.attrs);
    let gql_name = method_attrs
        .name
        .clone()
        .unwrap_or_else(|| to_camel_case(&method.sig.ident.to_string()));
    let desc = description(&method.attrs, &method_attrs.desc);
    let field_guard = guard_check(&method_attrs.guard, &quote! { rc.ctx })?;

    let parsed = parse_method_args(&mut method.sig)?;
    let method_ident = method.sig.ident.clone();
    let ret_ty: syn::Type = match &method.sig.output {
        syn::ReturnType::Type(_, ty) => (**ty).clone(),
        syn::ReturnType::Default => syn::parse_quote! { () },
    };

    let mut arg_bindings = Vec::new();
    let mut arg_calls = Vec::new();
    let mut register_arg_calls = Vec::new();
    // A resolver taking `ctx: &Context<'_>` had the parameter bound but
    // never forwarded to the actual method call - an arity mismatch that
    // surfaced as a confusing error once a method with this parameter
    // shape was actually exercised. `ctx` must go first, ahead of the
    // GraphQL args, matching where it's conventionally declared as the
    // first parameter after `&self`.
    let mut call_arg_idents = Vec::new();
    if let Some(ident) = &parsed.ctx_ident {
        call_arg_idents.push(ident.clone());
    }
    for arg in &parsed.args {
        let (binding, arg_call) = gen_arg(arg)?;
        arg_bindings.push(binding);
        arg_calls.push(arg_call);
        let ty = &arg.ty;
        register_arg_calls.push(quote! { <#ty as #crate_path::InputType>::register(registrar); });
        call_arg_idents.push(arg.ident.clone());
    }

    let ctx_binding = parsed
        .ctx_ident
        .as_ref()
        .map(|ident| quote! { let #ident = rc.ctx; });

    let desc_call = match &desc {
        Some(d) => quote! { .description(#d) },
        None => quote! {},
    };

    let add_field_stmt = quote! {
        let object = object.field(
            #crate_path::dynamic::Field::new(
                #gql_name,
                <#ret_ty as #crate_path::OutputType>::type_ref(),
                move |rc| {
                    #crate_path::dynamic::FieldFuture::Future(::std::boxed::Box::pin(async move {
                        #object_guard
                        #field_guard
                        #(#arg_bindings)*
                        #ctx_binding
                        #self_stmt
                        let __res = #self_ty::#method_ident(__self_ref, #(#call_arg_idents),*).await;
                        <#ret_ty as #crate_path::OutputType>::resolve_owned(__res)
                    }))
                },
            )
            #(#arg_calls)*
            #desc_call,
        );
    };
    let register_stmt = quote! {
        <#ret_ty as #crate_path::OutputType>::register(registrar);
        #(#register_arg_calls)*
    };
    Ok((add_field_stmt, register_stmt))
}

// ---------------------------------------------------------------------------
// `#[Subscription]` method -> `SubscriptionField` codegen
// ---------------------------------------------------------------------------

/// Pull `T` out of a `#[Subscription]` method's return type, which must be
/// `Result<impl Stream<Item = T>>` (`Result`'s own path is not checked
/// beyond its last segment — a real-world method might write
/// `async_graphql::Result<..>` while a port writes plain `Result<..>` via
/// `flash_graphql`'s re-export; either spelling works the same way
/// `gen_arg`/`gen_object_field` don't care which `Result` a method uses).
/// Matches a typical real-world `#[Subscription]` method shape exactly:
/// always a bare `impl Stream<Item = ..>`, never a named stream type.
pub(crate) fn extract_stream_item_ty(ret_ty: &syn::Type) -> MacroResult<syn::Type> {
    let unsupported = || {
        syn::Error::new(
            ret_ty.span(),
            "#[Subscription] methods must return `Result<impl Stream<Item = T>>`",
        )
        .into()
    };
    let syn::Type::Path(type_path) = ret_ty else {
        return Err(unsupported());
    };
    let Some(last) = type_path.path.segments.last() else {
        return Err(unsupported());
    };
    let syn::PathArguments::AngleBracketed(result_args) = &last.arguments else {
        return Err(unsupported());
    };
    let Some(inner_ty) = result_args.args.iter().find_map(|a| match a {
        syn::GenericArgument::Type(t) => Some(t),
        _ => None,
    }) else {
        return Err(unsupported());
    };
    let syn::Type::ImplTrait(impl_trait) = inner_ty else {
        return Err(unsupported());
    };
    for bound in &impl_trait.bounds {
        let syn::TypeParamBound::Trait(trait_bound) = bound else {
            continue;
        };
        let Some(seg) = trait_bound.path.segments.last() else {
            continue;
        };
        if seg.ident != "Stream" {
            continue;
        }
        let syn::PathArguments::AngleBracketed(stream_args) = &seg.arguments else {
            continue;
        };
        for sa in &stream_args.args {
            if let syn::GenericArgument::AssocType(assoc) = sa
                && assoc.ident == "Item"
            {
                return Ok(assoc.ty.clone());
            }
        }
    }
    Err(unsupported())
}

/// Turn one `#[Subscription]` `async fn` method into (a) a `let subscription
/// = subscription.field(..);` statement to fold into `SubscriptionRoot::
/// add_fields`, and (b) the matching `register` statement for its stream
/// item/argument types. Mirrors [`gen_object_field`] closely (same arg/guard/
/// self-value handling — `#[Subscription]`-annotated types are, like
/// `#[Object]`'s, always zero-sized marker structs in real-world usage, so
/// `self_stmt` constructs a fresh `Self::default()`, never a `parent_value`
/// downcast),
/// differing only in the shape of the field itself: a subscription field's
/// resolver produces a *stream* of values (`SubscriptionFieldFuture`), not a
/// single one (`FieldFuture`), so the user's method is `.await?`'d for its
/// `Result<impl Stream<Item = T>>` and then each yielded `T` is mapped
/// through `OutputType::resolve_owned` into the `Result<FieldValue<'static>>`
/// shape `dynamic::SubscriptionFieldFuture::new` requires (see
/// `flash-graphql`'s `futures_util` re-export doc comment for why `T`'s
/// `Into<FieldValue<'a>>` bound is satisfied by a plain `FieldValue<'static>`
/// item here, the same variance argument `gen_object_field`'s `resolve_owned`
/// splice already relies on).
pub(crate) fn gen_subscription_field(
    method: &mut syn::ImplItemFn,
    self_ty: &syn::Type,
    self_stmt: &TokenStream,
    object_guard: &Option<TokenStream>,
) -> MacroResult<(TokenStream, TokenStream)> {
    let crate_path = crate_path();
    let method_attrs: MethodAttrs = parse_graphql_attr(&method.attrs)?;
    remove_graphql_attrs(&mut method.attrs);
    let gql_name = method_attrs
        .name
        .clone()
        .unwrap_or_else(|| to_camel_case(&method.sig.ident.to_string()));
    let desc = description(&method.attrs, &method_attrs.desc);
    let field_guard = guard_check(&method_attrs.guard, &quote! { rc.ctx })?;

    let parsed = parse_method_args(&mut method.sig)?;
    let method_ident = method.sig.ident.clone();
    let ret_ty: syn::Type = match &method.sig.output {
        syn::ReturnType::Type(_, ty) => (**ty).clone(),
        syn::ReturnType::Default => {
            return Err(syn::Error::new(
                method.sig.span(),
                "#[Subscription] methods must return `Result<impl Stream<Item = T>>`",
            )
            .into());
        }
    };
    let item_ty = extract_stream_item_ty(&ret_ty)?;

    let mut arg_bindings = Vec::new();
    let mut arg_calls = Vec::new();
    let mut register_arg_calls = Vec::new();
    let mut call_arg_idents = Vec::new();
    if let Some(ident) = &parsed.ctx_ident {
        call_arg_idents.push(ident.clone());
    }
    for arg in &parsed.args {
        let (binding, arg_call) = gen_arg(arg)?;
        arg_bindings.push(binding);
        arg_calls.push(arg_call);
        let ty = &arg.ty;
        register_arg_calls.push(quote! { <#ty as #crate_path::InputType>::register(registrar); });
        call_arg_idents.push(arg.ident.clone());
    }

    let ctx_binding = parsed
        .ctx_ident
        .as_ref()
        .map(|ident| quote! { let #ident = rc.ctx; });

    let desc_call = match &desc {
        Some(d) => quote! { .description(#d) },
        None => quote! {},
    };

    let add_field_stmt = quote! {
        let subscription = subscription.field(
            #crate_path::dynamic::SubscriptionField::new(
                #gql_name,
                <#item_ty as #crate_path::OutputType>::type_ref(),
                move |rc| {
                    #crate_path::dynamic::SubscriptionFieldFuture::new(async move {
                        #object_guard
                        #field_guard
                        #(#arg_bindings)*
                        #ctx_binding
                        #self_stmt
                        let __stream = #self_ty::#method_ident(__self_ref, #(#call_arg_idents),*).await?;
                        ::std::result::Result::Ok(#crate_path::futures_util::StreamExt::map(
                            __stream,
                            |__item| {
                                <#item_ty as #crate_path::OutputType>::resolve_owned(__item)
                                    .map(|__opt| __opt.unwrap_or(#crate_path::dynamic::FieldValue::NULL))
                            },
                        ))
                    })
                },
            )
            #(#arg_calls)*
            #desc_call,
        );
    };
    let register_stmt = quote! {
        <#item_ty as #crate_path::OutputType>::register(registrar);
        #(#register_arg_calls)*
    };
    Ok((add_field_stmt, register_stmt))
}
