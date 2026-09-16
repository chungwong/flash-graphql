//! End-to-end test of `#[derive(OneofObject)]`, matching a typical
//! real-world oneof-input shape exactly (`enum TargetBy { Codes(Vec<String>),
//! Coordinates(Coordinates) }`) and its use as a mutation input field.
//!
//! Proves: (a) the SDL prints `input TargetBy @oneOf { .. }` with every
//! field nullable; (b) a single-field input parses to the right variant, for
//! either field; (c) a zero-field input is a clean GraphQL error (not a
//! panic, not a misrouted variant); (d) a both-fields-set input is likewise
//! a clean error. (c)/(d) are both enforced by the real dynamic engine's own
//! request validator (`oneof: true` on the registered `MetaType::InputObject`
//! — see `flash-graphql-derive`'s `oneof_object.rs` doc comment) rather than
//! by any hand-rolled check in this crate's generated `parse()`.

use flash_graphql::{InputObject, Object, OneofObject, Schema};

#[derive(Debug, InputObject)]
struct Coordinates {
    lat: Option<String>,
    lng: Option<String>,
}

#[derive(Debug, OneofObject)]
enum TargetBy {
    Codes(Vec<String>),
    Coordinates(Coordinates),
}

#[derive(Default)]
struct QueryRoot;

#[Object]
impl QueryRoot {
    async fn ping(&self) -> String {
        "pong".into()
    }
}

#[derive(Default)]
struct MutationRoot;

#[Object]
impl MutationRoot {
    async fn update(&self, target_by: TargetBy) -> String {
        match target_by {
            TargetBy::Codes(codes) => format!("codes:{}", codes.join(",")),
            TargetBy::Coordinates(c) => {
                format!(
                    "coords:{}/{}",
                    c.lat.unwrap_or_default(),
                    c.lng.unwrap_or_default()
                )
            }
        }
    }
}

fn build_schema() -> flash_graphql::Schema<QueryRoot, MutationRoot, flash_graphql::EmptySubscription>
{
    Schema::build(QueryRoot, MutationRoot, flash_graphql::EmptySubscription)
        .finish()
        .expect("schema should build")
}

#[tokio::test]
async fn sdl_shows_oneof_input() {
    let schema = build_schema();
    let sdl = schema.sdl();
    println!("--- SDL ---\n{sdl}");
    assert!(sdl.contains("input TargetBy @oneOf"), "sdl:\n{sdl}");
    assert!(sdl.contains("codes: [String!]"), "sdl:\n{sdl}");
    assert!(sdl.contains("coordinates: Coordinates"), "sdl:\n{sdl}");
}

#[tokio::test]
async fn valid_single_field_succeeds_for_either_variant() {
    let schema = build_schema();

    let q1 = r#"mutation { update(targetBy: { codes: ["A", "B"] }) }"#;
    let r1 = schema.execute(q1).await;
    assert!(r1.errors.is_empty(), "errors: {:#?}", r1.errors);
    let json: serde_json::Value = serde_json::to_value(&r1.data).unwrap();
    assert_eq!(json["update"], "codes:A,B");

    let q2 = r#"mutation { update(targetBy: { coordinates: { lat: "L" } }) }"#;
    let r2 = schema.execute(q2).await;
    assert!(r2.errors.is_empty(), "errors: {:#?}", r2.errors);
    let json: serde_json::Value = serde_json::to_value(&r2.data).unwrap();
    assert_eq!(json["update"], "coords:L/");
}

#[tokio::test]
async fn zero_fields_is_a_clean_error() {
    let schema = build_schema();
    let q = r#"mutation { update(targetBy: {}) }"#;
    let response = schema.execute(q).await;
    assert!(!response.errors.is_empty(), "expected a validation error");
    assert!(
        response.errors[0].message.contains("Oneof")
            || response.errors[0].message.contains("exactly one"),
        "{:#?}",
        response.errors
    );
}

#[tokio::test]
async fn multiple_fields_is_a_clean_error() {
    let schema = build_schema();
    let q = r#"mutation {
        update(targetBy: { codes: ["A"], coordinates: { lat: "L" } })
    }"#;
    let response = schema.execute(q).await;
    assert!(!response.errors.is_empty(), "expected a validation error");
    assert!(
        response.errors[0].message.contains("Oneof")
            || response.errors[0].message.contains("exactly one"),
        "{:#?}",
        response.errors
    );
}
