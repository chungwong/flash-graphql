//! `#[graphql(guard = "..")]` on a plain `SimpleObject` field — added
//! because real-world schemas have this shape (a field gated by a guard
//! with no `#[ComplexObject]` needed for that one field). Before this,
//! `FieldAttrs` silently dropped an unrecognized `guard` key (darling's
//! "Unexpected type"/"Unknown field" error), and since a plain field's
//! resolver is normally sync (`FieldFuture::Value`, see `simple_object.rs`'s
//! module doc) there was nowhere to `.await` a guard check at all. Proves
//! both the allow and deny paths, and that a field with no guard alongside
//! one that has it still resolves synchronously (no regression to the
//! common case).

use flash_graphql::{Context, Error, Guard, Object, Result, Schema, SimpleObject};

struct AlwaysDeny;

impl Guard for AlwaysDeny {
    async fn check(&self, _ctx: &Context<'_>) -> Result<()> {
        Err(Error::new("not allowed"))
    }
}

struct AlwaysAllow;

impl Guard for AlwaysAllow {
    async fn check(&self, _ctx: &Context<'_>) -> Result<()> {
        Ok(())
    }
}

#[derive(SimpleObject)]
struct Report {
    /// No guard at all — must keep resolving the plain, sync way.
    summary: String,
    #[graphql(guard = "AlwaysAllow")]
    allowed_section: String,
    #[graphql(guard = "AlwaysDeny")]
    denied_section: String,
}

#[derive(Default)]
struct QueryRoot;

#[Object]
impl QueryRoot {
    async fn report(&self) -> Report {
        Report {
            summary: "ok".into(),
            allowed_section: "visible".into(),
            denied_section: "should never come back".into(),
        }
    }
}

fn build_schema() -> Schema<QueryRoot> {
    Schema::build(QueryRoot, Default::default(), Default::default())
        .finish()
        .expect("schema should build")
}

#[tokio::test]
async fn ungated_field_resolves_normally() {
    let schema = build_schema();
    let response = schema.execute("{ report { summary } }").await;
    assert!(response.errors.is_empty(), "errors: {:#?}", response.errors);
    let json: serde_json::Value = serde_json::to_value(&response.data).unwrap();
    assert_eq!(json["report"]["summary"], "ok");
}

#[tokio::test]
async fn allow_guarded_field_still_resolves() {
    let schema = build_schema();
    let response = schema.execute("{ report { allowedSection } }").await;
    assert!(response.errors.is_empty(), "errors: {:#?}", response.errors);
    let json: serde_json::Value = serde_json::to_value(&response.data).unwrap();
    assert_eq!(json["report"]["allowedSection"], "visible");
}

#[tokio::test]
async fn deny_guarded_field_produces_a_real_graphql_error() {
    let schema = build_schema();
    let response = schema.execute("{ report { deniedSection } }").await;
    assert!(!response.errors.is_empty(), "expected a guard error");
    assert!(
        response.errors[0].message.contains("not allowed"),
        "{:#?}",
        response.errors
    );
}

#[tokio::test]
async fn a_field_without_a_guard_is_unaffected_by_a_sibling_that_has_one() {
    let schema = build_schema();
    let response = schema
        .execute("{ report { summary allowedSection } }")
        .await;
    assert!(response.errors.is_empty(), "errors: {:#?}", response.errors);
    let json: serde_json::Value = serde_json::to_value(&response.data).unwrap();
    assert_eq!(json["report"]["summary"], "ok");
    assert_eq!(json["report"]["allowedSection"], "visible");
}
