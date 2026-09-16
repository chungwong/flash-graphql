# Vendor patch: `async-graphql` 7.0.17 `dynamic` engine

This directory (`vendor/async-graphql-7.0.17/`) is a full local copy of the
`async-graphql` 7.0.17 crate (copied from the unmodified upstream source in
`~/.cargo/registry/src/*/async-graphql-7.0.17/`), wired in via
`[patch.crates-io]` in this workspace's `Cargo.toml` so `flash-graphql`
builds against it in place of the crates.io release. Only one file is
changed: `src/dynamic/resolve.rs`. Everything else — including
`async-graphql-derive`, `async-graphql-parser`, and `async-graphql-value`,
async-graphql's three companion crates — is left as an ordinary, unpinned
crates.io dependency; none of them needed a change for this patch.

## Why

`flash-graphql` wraps async-graphql's type-erased `dynamic` (code-first,
non-derive) engine for fast incremental compiles. That engine, as shipped on
crates.io, has two real deviations from spec-compliant GraphQL execution
that the STATIC (derive-macro) side of async-graphql already gets right:

1. **No nullable-error isolation.** Per the GraphQL spec (and per
   async-graphql's own static `Option<T>::resolve`, see
   `src/types/external/optional.rs:60-75` in the unpatched vendor copy), a
   resolver error on a *nullable* field should be recorded in the response's
   top-level `errors` array (with a `path`) and that one field's value
   should become `null` — while sibling fields and ancestor objects resolve
   normally. The dynamic engine instead propagates every resolver error via
   `?` in `dynamic::resolve::collect_field`, so a single failing nullable
   field null the *entire* response (`data: null`) exactly as if it were a
   non-nullable field.
2. **Serial, not concurrent, sibling field resolution.** `resolve_container`
   in `dynamic/resolve.rs` takes a `serial: bool` and is called with
   `serial: true` for every nested object/interface/union field set, so
   sibling fields on the same object resolve one after another. List
   elements are already resolved concurrently (`resolve_list` uses
   `try_join_all`), and so are the STATIC engine's object fields
   (`resolver_utils::container::resolve_container_inner`, always called with
   `parallel: true` for both `Query` and any nested container via
   `resolve_container`) — only the dynamic engine's *nested* object fields
   were left serial.

`flash-graphql`'s own derive macros (`crates/flash-graphql-derive`) do not
implement any error-catching or nullability logic of their own — each
generated field closure just calls the user's resolver and returns whatever
it returns. Both fixes therefore belong in the engine itself, matching
where the static engine's equivalent behaviour lives.

## What changed, and why it's correct

Both changes are inside `collect_field` and the three `resolve_container`
call sites in `resolve_value`, in `src/dynamic/resolve.rs`.

### 1. Nullable-error isolation

```rust
let field_value = match field_future {
    FieldFuture::Value(field_value) => field_value,
    FieldFuture::Future(future) => future.await.map_err(|err| {
        ctx_field.set_error_path(err.into_server_error(field.pos))
    })?,
};
```

Upstream converts a resolver's `Err` via `.into_server_error(field.pos)`
*without* stamping a `path` (confirmed by reading both this and the static
derive's equivalent, `async-graphql-derive`'s
`generate_field_resolver_method`: `f.await.map_err(|err| ctx.set_error_path(err))?`
— the static side stamps the path immediately, at the resolver's own field
context, before anything else happens to the error). This patch does the
same: `ctx_field` at this point is exactly the failing field's own context,
so the recorded `path` is correct at the moment of origin — including for
nested non-null propagation, where a deeper field's error passes *through*
several ancestor fields unmodified before it's finally isolated (see below);
whichever field's `collect_field` invocation first observes the error is
the one that stamps it, so an outer catch never overwrites a correct deep
path with its own shallower one.

```rust
let res_value = match ctx_field
    .query_env
    .extensions
    .resolve(resolve_info, &mut resolve_fut)
    .await
{
    Ok(value) => value.unwrap_or_default(),
    Err(err) if field_def.ty.is_nullable() => {
        ctx_field.add_error(err);
        Value::Null
    }
    Err(err) => return Err(err),
};
```

`field_def.ty` is this field's own `TypeRef` (already `pub(crate)
is_nullable()` on `TypeRef`, no new API needed). If the field is nullable,
any error surfacing from resolving it — whether the resolver call itself
failed (caught above) or a deeper, non-null child field's error propagated
up through it — is recorded via `ctx_field.add_error` (the same public
method `Option<T>::resolve` calls on the static side) and this field's own
value becomes `Value::Null`. Extensions, the `resolve_info` span, and
everything else are untouched. If the field is *non-null*, the `Err` keeps
propagating via `return Err(err)` — spec-correct: null in a non-null
position becomes an error on the nearest nullable ancestor (or the whole
response, at the root — this exactly matches pre-patch behaviour for that
one case, since the root has no nullable ancestor above it either).

Argument coercion (the `ObjectAccessor` construction earlier in the same
function) is deliberately left untouched — a coercion failure happens
before the resolver is even invoked and isn't part of what either spec
text or the static engine treats as a "resolver error" for isolation
purposes.

### 2. Concurrent sibling field resolution

Three identical-shape call sites in `resolve_value` (`Type::Object`,
`Type::Interface`, `Type::Union`) changed their last argument to
`resolve_container` from `true` to `false`:

```rust
resolve_container(
    schema,
    object, // or object_type, for interface/union dispatch
    &ctx.with_selection_set(&ctx.item.node.selection_set),
    value,
    false, // was: true
)
.await
```

`resolve_container`'s `!serial` branch (`futures_util::future::try_join_all`)
already existed and is already exercised in production: it's the same code
path `dynamic/schema.rs` uses for the root `Query` object (`serial: false`
there already; only `Mutation`, correctly, stays serial via
`resolve_container(.., true)`, matching the spec's requirement that
mutation root fields run in order). This patch does not add a new
concurrency mechanism — it just applies the existing, already-proven one to
nested object fields too, closing the one gap between the dynamic engine's
nested-object behaviour and both (a) its own root-Query behaviour and (b)
the static engine's `resolver_utils::container::resolve_container`, which
always resolves object fields concurrently regardless of nesting depth.

## Was concurrent resolution actually safe here? (guard semantics)

Yes — verified before flipping the flag, not assumed:

- Every field's guard chain runs entirely inside that field's own
  `FieldFuture::Future` closure (see
  `crates/flash-graphql-derive/src/util.rs::gen_object_field`: guard check,
  argument binding, and the resolver call are all sequenced inside one
  `async move { .. }` block generated per field). Two sibling fields share
  no guard-related mutable state — a guard denying field B has no way to
  observe, delay, or affect field A's own guard check or resolution, because
  they are two independent futures with no data dependency between them.
- The only *shared* state reachable from concurrent sibling futures is
  `query_env` (data, extensions, the `errors: Mutex<Vec<ServerError>>`) —
  already designed for concurrent access, since the root `Query` object's
  fields (and `DataLoader` batching across list elements, and now sibling
  object fields) already run concurrently against it today.
- Root-level Query field concurrency is not a new pattern this patch
  introduces at a deeper level for the first time — it's the *existing*,
  already-shipped default behaviour of every async-graphql schema (both
  engines), just extended one level of nesting further to be consistent.

No guard-ordering or auth-check correctness issue was found. This is
reflected in `crates/flash-graphql/tests/partial_response_smoke.rs`, which
sanity-checked (temporarily, during development, not committed) that its
concurrency assertion actually fails under the old `serial: true` behaviour
and passes only with the patch — i.e. the test is not vacuous.

## Diff

```diff
--- async-graphql-7.0.17/src/dynamic/resolve.rs (unpatched, upstream)
+++ async-graphql-7.0.17/src/dynamic/resolve.rs (flash-graphql vendor patch)
@@ -261,9 +261,22 @@
 
                 let field_value = match field_future {
                     FieldFuture::Value(field_value) => field_value,
-                    FieldFuture::Future(future) => future
-                        .await
-                        .map_err(|err| err.into_server_error(field.pos))?,
+                    FieldFuture::Future(future) => future.await.map_err(|err| {
+                        // flash-graphql patch (nullable-error isolation, part 1/2):
+                        // stamp the path *here*, at this field's own context,
+                        // the moment a resolver-raised error is first observed.
+                        // This mirrors what async-graphql's STATIC derive
+                        // codegen does for `#[Object]` methods
+                        // (`generate_field_resolver_method` in
+                        // async-graphql-derive: `f.await.map_err(|err| ctx.set_error_path(err))?`)
+                        // *before* propagating, so that if this error later
+                        // gets isolated at a nullable ancestor (see part 2/2
+                        // below, or an ancestor field further up the tree),
+                        // the recorded `path` still points at the field that
+                        // actually failed instead of the ancestor that
+                        // happened to catch it.
+                        ctx_field.set_error_path(err.into_server_error(field.pos))
+                    })?,
                 };
 
                 let value =
@@ -273,12 +286,32 @@
             };
             futures_util::pin_mut!(resolve_fut);
 
-            let res_value = ctx_field
+            // flash-graphql patch (nullable-error isolation, part 2/2): upstream
+            // async-graphql's dynamic engine propagates every field error all the
+            // way to the root via `?` here, so one failing NULLABLE field nulls
+            // the entire response instead of just nulling itself (see
+            // vendor/PATCH.md). Spec-compliant behaviour, and what the STATIC
+            // engine already does via `Option<T>::resolve`
+            // (async-graphql/src/types/external/optional.rs): a nullable field's
+            // error is recorded in `errors` (with `path`, stamped above or by
+            // whichever context first observed the error) and the field's own
+            // value becomes `null`, while sibling fields and ancestor objects
+            // still resolve normally. A NON-nullable field keeps propagating
+            // unchanged (`?`) — that's spec-correct: null in a non-null position
+            // becomes an error on the nearest nullable ancestor.
+            let res_value = match ctx_field
                 .query_env
                 .extensions
                 .resolve(resolve_info, &mut resolve_fut)
-                .await?
-                .unwrap_or_default();
+                .await
+            {
+                Ok(value) => value.unwrap_or_default(),
+                Err(err) if field_def.ty.is_nullable() => {
+                    ctx_field.add_error(err);
+                    Value::Null
+                }
+                Err(err) => return Err(err),
+            };
             Ok((field.node.response_key().node.clone(), res_value))
         }
         .boxed(),
@@ -505,12 +538,22 @@
         )),
 
         (Type::Object(object), _) => {
+            // flash-graphql patch (concurrent sibling resolution): upstream
+            // hardcodes `serial: true` here, so sibling fields on the same
+            // object resolve one after another instead of concurrently
+            // (unlike list elements, which `resolve_list` above already
+            // resolves via `try_join_all`, and unlike the STATIC engine's
+            // `resolver_utils::container::resolve_container`, which always
+            // uses `parallel: true` / `try_join_all` for object fields — see
+            // vendor/PATCH.md). Flipped to `false` (concurrent), reusing the
+            // `try_join_all` path this same function already has for the
+            // root-level Query call in `dynamic/schema.rs`.
             resolve_container(
                 schema,
                 object,
                 &ctx.with_selection_set(&ctx.item.node.selection_set),
                 value,
-                true,
+                false,
             )
             .await
         }
@@ -587,12 +630,16 @@
                     )
                 })?;
 
+            // flash-graphql patch (concurrent sibling resolution): see the
+            // `Type::Object` arm above and vendor/PATCH.md — same change,
+            // applied to the object a resolved interface/union value
+            // dispatches to.
             resolve_container(
                 schema,
                 object_type,
                 &ctx.with_selection_set(&ctx.item.node.selection_set),
                 value,
-                true,
+                false,
             )
             .await
         }
@@ -633,12 +680,16 @@
                     )
                 })?;
 
+            // flash-graphql patch (concurrent sibling resolution): see the
+            // `Type::Object` arm above and vendor/PATCH.md — same change,
+            // applied to the object a resolved interface/union value
+            // dispatches to.
             resolve_container(
                 schema,
                 object_type,
                 &ctx.with_selection_set(&ctx.item.node.selection_set),
                 value,
-                true,
+                false,
             )
             .await
         }
```

(This is the literal diff between the pristine registry copy at
`~/.cargo/registry/src/*/async-graphql-7.0.17/src/dynamic/resolve.rs` and
`vendor/async-graphql-7.0.17/src/dynamic/resolve.rs` in this repo — 4
hunks, ~50 net new lines, all comments plus two behavioural changes:
one `.map_err` closure body, one `let ... = ctx_field.query_env.extensions...`
turned into a `match`, and three `true` → `false` literals.)

## Proof

`crates/flash-graphql/tests/partial_response_smoke.rs` proves both fixes
end-to-end against a real `flash_graphql::Schema`:

- `nullable_field_error_is_isolated_from_its_siblings` — a nullable field's
  resolver errors; the response has exactly one error with
  `path: ["nullableBoom"]`, that field's value is `null`, and a sibling
  field's real value is still present.
- `non_nullable_field_error_with_no_nullable_ancestor_nulls_the_whole_response`
  and `non_nullable_field_error_nulls_only_the_nearest_nullable_ancestor` —
  a non-nullable field's resolver errors; the error propagates to the
  nearest nullable ancestor (a wrapping `Option<Parent>` field, which
  becomes `null` while a root sibling field is untouched) or, with no
  nullable ancestor at all, nulls the entire response — proving this half
  of pre-patch behaviour did not regress.
- `sibling_fields_on_one_object_resolve_concurrently` — two sibling fields
  on one nested object each record a `(start, end)` timestamp pair around a
  real 60ms async delay; their intervals overlap, which is only possible
  under concurrent resolution.

All pre-existing tests across the workspace (`cargo test --workspace`) still
pass unchanged; none of them asserted on the old (propagate-everything,
serial) behaviour, so no test expectations needed to change. One stale doc
comment (in `tests/dataloader_smoke.rs`, describing nested object fields as
serial) was updated to match.

## Upstreaming

This is a real, general-purpose correctness fix to async-graphql's public
`dynamic` engine, not something specific to flash-graphql — genuinely worth
proposing upstream. Notes for whoever does that:

- Upstream repository: <https://github.com/async-graphql/async-graphql>
  (from this crate's own `Cargo.toml`). There is no `CONTRIBUTING.md` in
  the crate; the repo uses standard GitHub issue templates
  (`.github/ISSUE_TEMPLATE/{bug_report,feature_request,question}.md`) and,
  presumably, ordinary GitHub pull requests reviewed by the maintainers
  (`sunli`/`Koxiaet` per `Cargo.toml`'s `authors`).
- The nullable-isolation fix is a strict correctness improvement matching
  the engine's own static-derive behaviour and documented GraphQL spec
  semantics — should be uncontroversial, but touches error-handling
  behavior other users may have adapted to (however unlikely, since the
  current behaviour is a known-bad gap between the two engines the crate
  ships), so it's the more discussion-worthy of the two changes.
- The concurrency fix is lower-risk: it only makes dynamic-engine nested
  object field resolution match (a) the dynamic engine's own root-Query
  behaviour and (b) the static engine's behaviour at every nesting depth;
  it doesn't introduce any new concurrency primitive.
- Nobody has opened a PR against upstream from this project yet; this
  document is preparation for that, not a substitute for it.
