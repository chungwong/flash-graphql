//! Proves the flash-graphql vendor patch to async-graphql 7.0.17's `dynamic`
//! engine (see `vendor/PATCH.md`, applied in
//! `vendor/async-graphql-7.0.17/src/dynamic/resolve.rs`):
//!
//! 1. A NULLABLE field's resolver error is isolated: recorded in the
//!    response's `errors` (with a `path` pointing at the failing field), the
//!    field's own value becomes `null` in `data` — but a sibling field still
//!    resolves normally
//!    (`nullable_field_error_is_isolated_from_its_siblings`). This is the
//!    core proof that partial responses now work.
//! 2. A NON-nullable field's resolver error still propagates — to the
//!    nearest nullable ancestor
//!    (`non_nullable_field_error_nulls_only_the_nearest_nullable_ancestor`),
//!    or, with no nullable ancestor above it, the whole response
//!    (`non_nullable_field_error_with_no_nullable_ancestor_nulls_the_whole_response`)
//!    — unchanged from pre-patch/spec behaviour. Proves (1) didn't regress
//!    this half of the contract.
//! 3. Sibling fields on the same object resolve *concurrently*, not one
//!    after another (`sibling_fields_on_one_object_resolve_concurrently`).

use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use flash_graphql::{ComplexObject, Error, Object, Result, Schema, SimpleObject};
use tokio::time::sleep;

#[derive(Default)]
struct QueryRoot;

#[Object]
impl QueryRoot {
    /// A perfectly healthy field, queried alongside a failing one below.
    async fn ok(&self) -> String {
        "fine".into()
    }

    /// Nullable (`Result<Option<String>>` blanket-impls to GraphQL `String`,
    /// no `!` — see `output.rs`'s `Option<T>`/`Result<T, E>` impls): errors,
    /// must be isolated to just this field.
    async fn nullable_boom(&self) -> Result<Option<String>> {
        Err(Error::new("nullable boom"))
    }

    /// Non-nullable (`Result<String>` -> GraphQL `String!`): errors, must
    /// propagate. There is no nullable ancestor above a root query field, so
    /// this nulls the whole response — matching pre-patch behaviour for
    /// this one case exactly (spec: null in a non-null position propagates
    /// to the nearest nullable ancestor, or the whole response at the root).
    async fn nonnull_boom(&self) -> Result<String> {
        Err(Error::new("non-null boom"))
    }

    /// A nullable parent wrapping a non-null child that errors: the error
    /// must propagate up to `parent` (its nearest nullable ancestor) and
    /// stop there, leaving `parent` itself `null` while sibling root fields
    /// still resolve.
    async fn parent(&self) -> Option<Parent> {
        Some(Parent)
    }

    async fn sibling(&self) -> Sibling {
        Sibling { id: 1 }
    }
}

#[derive(Default)]
struct Parent;

#[Object]
impl Parent {
    async fn ok_child(&self) -> String {
        "would resolve fine on its own".into()
    }

    /// Non-nullable: errors, must propagate to `parent` (the nearest
    /// nullable ancestor) and stop there — not all the way to the root.
    async fn bad_child(&self) -> Result<String> {
        Err(Error::new("child boom"))
    }
}

// ---------------------------------------------------------------------------
// Concurrency proof: two sibling fields on ONE object, each recording a
// (start, end) `Instant` pair around a real async delay. If sibling field
// resolution were still serial (pre-patch: `resolve_container(.., true)`),
// the two intervals could never overlap; concurrent resolution
// (`try_join_all`, matching `resolve_list` and the STATIC engine's
// `resolve_container`) makes them overlap.
// ---------------------------------------------------------------------------

static TIMINGS: Mutex<Vec<(&'static str, Instant, Instant)>> = Mutex::new(Vec::new());

async fn timed(name: &'static str) -> String {
    let start = Instant::now();
    sleep(Duration::from_millis(60)).await;
    let end = Instant::now();
    TIMINGS.lock().unwrap().push((name, start, end));
    name.to_string()
}

#[derive(SimpleObject)]
#[graphql(complex)]
struct Sibling {
    id: i32,
}

#[ComplexObject]
impl Sibling {
    async fn slow_a(&self) -> String {
        timed("a").await
    }

    async fn slow_b(&self) -> String {
        timed("b").await
    }
}

fn build_schema() -> Schema<QueryRoot> {
    Schema::build(QueryRoot, Default::default(), Default::default())
        .finish()
        .expect("schema should build")
}

#[tokio::test]
async fn nullable_field_error_is_isolated_from_its_siblings() {
    let schema = build_schema();
    let response = schema.execute("{ ok nullableBoom }").await;

    assert_eq!(response.errors.len(), 1, "errors: {:#?}", response.errors);
    assert!(
        response.errors[0].message.contains("nullable boom"),
        "{:#?}",
        response.errors
    );
    let path = serde_json::to_value(&response.errors[0].path).unwrap();
    assert_eq!(path, serde_json::json!(["nullableBoom"]), "path: {path:#?}");

    let json: serde_json::Value = serde_json::to_value(&response.data).unwrap();
    assert_eq!(
        json["ok"], "fine",
        "sibling field must still resolve: {json:#?}"
    );
    assert!(json["nullableBoom"].is_null(), "data: {json:#?}");
}

#[tokio::test]
async fn non_nullable_field_error_with_no_nullable_ancestor_nulls_the_whole_response() {
    let schema = build_schema();
    let response = schema.execute("{ ok nonnullBoom }").await;

    assert_eq!(response.errors.len(), 1, "errors: {:#?}", response.errors);
    assert!(
        response.errors[0].message.contains("non-null boom"),
        "{:#?}",
        response.errors
    );

    let json: serde_json::Value = serde_json::to_value(&response.data).unwrap();
    assert!(
        json.is_null(),
        "a non-null field's error with no nullable ancestor must null the \
         whole response (pre-patch behaviour, unchanged): {json:#?}"
    );
}

#[tokio::test]
async fn non_nullable_field_error_nulls_only_the_nearest_nullable_ancestor() {
    let schema = build_schema();
    let response = schema.execute("{ ok parent { okChild badChild } }").await;

    assert_eq!(response.errors.len(), 1, "errors: {:#?}", response.errors);
    assert!(
        response.errors[0].message.contains("child boom"),
        "{:#?}",
        response.errors
    );
    let path = serde_json::to_value(&response.errors[0].path).unwrap();
    assert_eq!(
        path,
        serde_json::json!(["parent", "badChild"]),
        "path: {path:#?}"
    );

    let json: serde_json::Value = serde_json::to_value(&response.data).unwrap();
    assert_eq!(
        json["ok"], "fine",
        "sibling ROOT field must still resolve: {json:#?}"
    );
    assert!(
        json["parent"].is_null(),
        "parent (nearest nullable ancestor) must be null, not the whole \
         response: {json:#?}"
    );
}

#[tokio::test]
async fn sibling_fields_on_one_object_resolve_concurrently() {
    TIMINGS.lock().unwrap().clear();
    let schema = build_schema();
    let response = schema.execute("{ sibling { slowA slowB } }").await;
    assert!(response.errors.is_empty(), "errors: {:#?}", response.errors);

    let timings = TIMINGS.lock().unwrap().clone();
    assert_eq!(timings.len(), 2, "timings: {timings:#?}");
    let (_, a_start, a_end) = *timings.iter().find(|(name, ..)| *name == "a").unwrap();
    let (_, b_start, b_end) = *timings.iter().find(|(name, ..)| *name == "b").unwrap();

    let overlap = a_start < b_end && b_start < a_end;
    assert!(
        overlap,
        "expected concurrent sibling field resolution to overlap in time: \
         a=[{a_start:?}, {a_end:?}], b=[{b_start:?}, {b_end:?}]"
    );
}
