//! A `#[graphql(complex)]` `SimpleObject` that is *also*
//! `#[graphql(flatten)]`ed into another struct must keep its `#[ComplexObject]`
//! fields in the outer type's SDL and resolution — found via a real
//! ORM-entity-backed type that was both `#[graphql(complex)]` and flattened
//! into an outer struct, whose exported SDL was silently missing its
//! complex fields before this was wired through
//! (`ComplexObjectFields::add_complex_fields` gained a `P`/
//! `project` parameter mirroring `Flatten`'s own, and `SimpleObject`'s
//! `Flatten` impl now calls it instead of only the direct, non-flattened
//! `OutputType::register` path doing so).

use flash_graphql::{
    ComplexObject, EmptyMutation, EmptySubscription, Object, Schema, SimpleObject,
};

/// The inner, complex-and-flattened type.
#[derive(Clone, Debug, SimpleObject)]
#[graphql(complex)]
struct Inner {
    plain: String,
}

#[ComplexObject]
impl Inner {
    async fn computed(&self) -> String {
        format!("computed:{}", self.plain)
    }
}

/// The outer type, flattening `Inner` in alongside its own field.
#[derive(Clone, Debug, SimpleObject)]
struct Outer {
    own_field: String,
    #[graphql(flatten)]
    inner: Inner,
}

#[derive(Default)]
struct QueryRoot;

#[Object]
impl QueryRoot {
    async fn outer(&self) -> Outer {
        Outer {
            own_field: "own".into(),
            inner: Inner {
                plain: "plain".into(),
            },
        }
    }
}

fn build_schema() -> Schema<QueryRoot, EmptyMutation, EmptySubscription> {
    Schema::build(QueryRoot, EmptyMutation, EmptySubscription)
        .finish()
        .expect("schema should build")
}

#[tokio::test]
async fn a_flattened_complex_type_s_computed_field_appears_in_the_sdl() {
    let schema = build_schema();
    let sdl = schema.sdl();
    assert!(sdl.contains("computed: String!"), "sdl:\n{sdl}");
    // `Inner` itself must never appear as its own GraphQL type — flatten
    // means its fields fold directly into `Outer`.
    assert!(!sdl.contains("type Inner"), "sdl:\n{sdl}");
}

#[tokio::test]
async fn a_flattened_complex_type_s_computed_field_resolves() {
    let schema = build_schema();
    let response = schema
        .execute("{ outer { ownField plain computed } }")
        .await;
    assert!(response.errors.is_empty(), "errors: {:#?}", response.errors);
    let json: serde_json::Value = serde_json::to_value(&response.data).unwrap();
    assert_eq!(json["outer"]["ownField"], "own");
    assert_eq!(json["outer"]["plain"], "plain");
    assert_eq!(json["outer"]["computed"], "computed:plain");
}
