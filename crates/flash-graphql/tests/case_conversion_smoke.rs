//! Default GraphQL-name case conversion edge cases the derives got wrong
//! until they were found via a real SDL diff against a production schema:
//!
//! - A snake_case field name whose last segment is a digit run followed
//!   directly by a letter (`printed_under_1d`) must camelCase to
//!   `printedUnder1D` (capitalizing the trailing letter as its own word),
//!   not `printedUnder1d`.
//! - A `PascalCase` enum variant with an embedded digit run
//!   (`From0To1Day`) must screaming-snake-case to `FROM_0_TO_1_DAY` (an
//!   underscore on *both* sides of the digit run), not `FROM0_TO1_DAY`.
//! - A raw identifier (`r#in`, a Rust keyword used as a field name) must
//!   lose its `r#` escape in the default GraphQL name (`in`), not keep it
//!   (`rIn`/`r#in`).

use flash_graphql::{Enum, InputObject, Object, Schema, SimpleObject};

#[derive(Debug, Clone, Copy, Enum, Eq, PartialEq)]
enum Range {
    From0To1Day,
    From20PlusDays,
}

#[derive(SimpleObject)]
struct Row {
    printed_under_1d: i32,
    synced_3d_plus: i32,
    active_month1: f64,
}

#[derive(Debug, InputObject)]
struct IntFilter {
    r#in: Option<Vec<i32>>,
}

#[derive(Default)]
struct QueryRoot;

#[Object]
impl QueryRoot {
    async fn range(&self) -> Range {
        Range::From0To1Day
    }

    async fn row(&self) -> Row {
        Row {
            printed_under_1d: 1,
            synced_3d_plus: 2,
            active_month1: 3.0,
        }
    }

    async fn filtered(&self, filter: IntFilter) -> i32 {
        filter.r#in.map_or(0, |v| v.len() as i32)
    }
}

fn build_schema() -> Schema<QueryRoot> {
    Schema::build(QueryRoot, Default::default(), Default::default())
        .finish()
        .expect("schema should build")
}

#[test]
fn digit_letter_boundaries_camel_case_correctly() {
    let sdl = build_schema().sdl();
    assert!(sdl.contains("printedUnder1D: Int!"), "sdl:\n{sdl}");
    assert!(sdl.contains("synced3DPlus: Int!"), "sdl:\n{sdl}");
    // A trailing-digit word (nothing follows the digit) is unaffected.
    assert!(sdl.contains("activeMonth1: Float!"), "sdl:\n{sdl}");
}

#[test]
fn digit_letter_boundaries_screaming_snake_case_correctly() {
    let sdl = build_schema().sdl();
    assert!(sdl.contains("FROM_0_TO_1_DAY"), "sdl:\n{sdl}");
    assert!(sdl.contains("FROM_20_PLUS_DAYS"), "sdl:\n{sdl}");
}

#[test]
fn a_raw_identifier_loses_its_escape_in_the_default_name() {
    let sdl = build_schema().sdl();
    assert!(sdl.contains("in: [Int!]"), "sdl:\n{sdl}");
    assert!(!sdl.contains("r#in"), "sdl:\n{sdl}");
    assert!(!sdl.contains("rIn"), "sdl:\n{sdl}");
}

#[tokio::test]
async fn the_raw_identifier_field_still_parses_correctly() {
    let schema = build_schema();
    let response = schema
        .execute("{ filtered(filter: { in: [1, 2, 3] }) }")
        .await;
    assert!(response.errors.is_empty(), "errors: {:#?}", response.errors);
    let json: serde_json::Value = serde_json::to_value(&response.data).unwrap();
    assert_eq!(json["filtered"], 3);
}
