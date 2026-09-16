//! End-to-end test of `flash_graphql::connection`: builds a real
//! `Connection<String, Foo, FooConnectionData>` type through
//! `#[derive(SimpleObject)]` for the node/additional-fields types and the
//! hand-written `connection::query` helper for the resolver itself — proving
//! the SDL shape (`edges`/`nodes`/`pageInfo` + a flattened extra field) and
//! that cursors/`pageInfo` come back correctly populated end to end, forward
//! and backward.

use flash_graphql::{
    ID, InputType, OutputType, RootFields, Schema, SimpleObject,
    connection::{self, Connection, Edge},
    dynamic::{self, FieldFuture},
};

#[derive(SimpleObject, Clone)]
struct Foo {
    id: ID,
    name: String,
}

/// Additional (flattened) fields on `FooConnection` — a typical
/// `ConnectionData { total: i64 }` shape.
#[derive(SimpleObject, Clone)]
struct FooConnectionData {
    total: i64,
}

fn all_foos() -> Vec<Foo> {
    (0..5)
        .map(|i| Foo {
            id: ID(i.to_string()),
            name: format!("foo{i}"),
        })
        .collect()
}

type FooConnection = Connection<String, Foo, FooConnectionData>;

async fn fetch_foos(
    after: Option<String>,
    before: Option<String>,
    first: Option<i32>,
    last: Option<i32>,
) -> flash_graphql::Result<FooConnection> {
    let foos = all_foos();
    let total = foos.len() as i64;

    connection::query(
        after,
        before,
        first,
        last,
        |after: Option<String>,
         before: Option<String>,
         first: Option<usize>,
         last: Option<usize>| async move {
            let mut start = after
                .and_then(|a| a.parse::<usize>().ok())
                .map(|n| n + 1)
                .unwrap_or(0);
            let mut end = before
                .and_then(|b| b.parse::<usize>().ok())
                .unwrap_or(foos.len());
            if let Some(first) = first {
                end = (start + first).min(end);
            }
            if let Some(last) = last {
                start = end.saturating_sub(last).max(start);
            }

            let mut conn = Connection::with_additional_fields(
                start > 0,
                end < foos.len(),
                FooConnectionData { total },
            );
            conn.edges.extend(
                foos[start..end]
                    .iter()
                    .cloned()
                    .enumerate()
                    .map(|(i, node)| Edge::new((start + i).to_string(), node)),
            );
            Ok::<_, flash_graphql::Error>(conn)
        },
    )
    .await
}

struct QueryRoot;

impl RootFields for QueryRoot {
    const NAME: &'static str = "Query";

    fn add_fields(object: dynamic::Object) -> dynamic::Object {
        object.field(
            dynamic::Field::new("foos", <FooConnection as OutputType>::type_ref(), |rc| {
                FieldFuture::Future(Box::pin(async move {
                    let after = <Option<String> as InputType>::parse(
                        rc.args.get("after").map(|v| v.as_value().clone()),
                    )?;
                    let before = <Option<String> as InputType>::parse(
                        rc.args.get("before").map(|v| v.as_value().clone()),
                    )?;
                    let first = <Option<i32> as InputType>::parse(
                        rc.args.get("first").map(|v| v.as_value().clone()),
                    )?;
                    let last = <Option<i32> as InputType>::parse(
                        rc.args.get("last").map(|v| v.as_value().clone()),
                    )?;
                    let conn = fetch_foos(after, before, first, last).await?;
                    <FooConnection as OutputType>::resolve_owned(conn)
                }))
            })
            .argument(dynamic::InputValue::new(
                "after",
                <Option<String> as InputType>::type_ref(),
            ))
            .argument(dynamic::InputValue::new(
                "before",
                <Option<String> as InputType>::type_ref(),
            ))
            .argument(dynamic::InputValue::new(
                "first",
                <Option<i32> as InputType>::type_ref(),
            ))
            .argument(dynamic::InputValue::new(
                "last",
                <Option<i32> as InputType>::type_ref(),
            )),
        )
    }

    fn register(registrar: &mut flash_graphql::Registrar) {
        <FooConnection as OutputType>::register(registrar);
        <Option<String> as InputType>::register(registrar);
        <Option<i32> as InputType>::register(registrar);
    }
}

fn build_schema() -> Schema<QueryRoot> {
    Schema::build(
        QueryRoot,
        flash_graphql::EmptyMutation,
        flash_graphql::EmptySubscription,
    )
    .finish()
    .expect("schema should build")
}

#[test]
fn sdl_shape_matches_real_relay_connections() {
    let sdl = build_schema().sdl();
    println!("--- SDL ---\n{sdl}");

    assert!(sdl.contains("type FooConnection {"), "sdl:\n{sdl}");
    assert!(sdl.contains("edges: [FooEdge!]!"), "sdl:\n{sdl}");
    assert!(sdl.contains("nodes: [Foo!]!"), "sdl:\n{sdl}");
    assert!(sdl.contains("pageInfo: PageInfo!"), "sdl:\n{sdl}");
    // The flattened additional field must appear directly on the connection,
    // not nested under its own field.
    assert!(sdl.contains("total: Int!"), "sdl:\n{sdl}");

    assert!(
        sdl.contains("\"\"\"\nAn edge in a connection.\n\"\"\"\ntype FooEdge {"),
        "sdl:\n{sdl}"
    );
    assert!(sdl.contains("node: Foo!"), "sdl:\n{sdl}");
    assert!(sdl.contains("cursor: String!"), "sdl:\n{sdl}");

    assert!(sdl.contains("type PageInfo {"), "sdl:\n{sdl}");
    assert!(sdl.contains("hasPreviousPage: Boolean!"), "sdl:\n{sdl}");
    assert!(sdl.contains("hasNextPage: Boolean!"), "sdl:\n{sdl}");
    assert!(sdl.contains("startCursor: String\n"), "sdl:\n{sdl}");
    assert!(sdl.contains("endCursor: String\n"), "sdl:\n{sdl}");
    // `PageInfo` is shared/non-generic: only one such type in the schema.
    assert_eq!(sdl.matches("type PageInfo {").count(), 1, "sdl:\n{sdl}");
}

#[tokio::test]
async fn forward_pagination_returns_correct_cursors_and_page_info() {
    let schema = build_schema();

    let query = r#"{
        foos(first: 2) {
            pageInfo { hasPreviousPage hasNextPage startCursor endCursor }
            edges { cursor node { id name } }
            nodes { name }
            total
        }
    }"#;
    let response = schema.execute(query).await;
    assert!(response.errors.is_empty(), "errors: {:#?}", response.errors);
    let json: serde_json::Value = serde_json::to_value(&response.data).unwrap();
    println!(
        "--- forward ---\n{}",
        serde_json::to_string_pretty(&json).unwrap()
    );

    let foos = &json["foos"];
    assert_eq!(foos["total"], 5);
    assert_eq!(foos["pageInfo"]["hasPreviousPage"], false);
    assert_eq!(foos["pageInfo"]["hasNextPage"], true);
    assert_eq!(foos["pageInfo"]["startCursor"], "0");
    assert_eq!(foos["pageInfo"]["endCursor"], "1");

    let edges = foos["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 2);
    assert_eq!(edges[0]["cursor"], "0");
    assert_eq!(edges[0]["node"]["name"], "foo0");
    assert_eq!(edges[1]["cursor"], "1");
    assert_eq!(edges[1]["node"]["name"], "foo1");

    let nodes = foos["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0]["name"], "foo0");
    assert_eq!(nodes[1]["name"], "foo1");
}

#[tokio::test]
async fn after_cursor_resumes_from_the_right_offset() {
    let schema = build_schema();

    let query = r#"{ foos(after: "1", first: 2) { pageInfo { hasPreviousPage hasNextPage } edges { cursor } } }"#;
    let response = schema.execute(query).await;
    assert!(response.errors.is_empty(), "errors: {:#?}", response.errors);
    let json: serde_json::Value = serde_json::to_value(&response.data).unwrap();

    let foos = &json["foos"];
    assert_eq!(foos["pageInfo"]["hasPreviousPage"], true);
    assert_eq!(foos["pageInfo"]["hasNextPage"], true);
    let edges = foos["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 2);
    assert_eq!(edges[0]["cursor"], "2");
    assert_eq!(edges[1]["cursor"], "3");
}

#[tokio::test]
async fn backward_pagination_with_last() {
    let schema = build_schema();

    let query = r#"{ foos(last: 2) { pageInfo { hasPreviousPage hasNextPage } edges { cursor node { name } } } }"#;
    let response = schema.execute(query).await;
    assert!(response.errors.is_empty(), "errors: {:#?}", response.errors);
    let json: serde_json::Value = serde_json::to_value(&response.data).unwrap();

    let foos = &json["foos"];
    assert_eq!(foos["pageInfo"]["hasPreviousPage"], true);
    assert_eq!(foos["pageInfo"]["hasNextPage"], false);
    let edges = foos["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 2);
    assert_eq!(edges[0]["node"]["name"], "foo3");
    assert_eq!(edges[1]["node"]["name"], "foo4");
}

#[tokio::test]
async fn negative_first_is_a_clear_error() {
    let schema = build_schema();
    let response = schema.execute(r#"{ foos(first: -1) { total } }"#).await;
    assert!(
        !response.errors.is_empty(),
        "expected an error for negative `first`"
    );
    assert!(
        response.errors[0].message.contains("non-negative"),
        "{:#?}",
        response.errors
    );
}
