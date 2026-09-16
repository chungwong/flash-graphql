//! End-to-end test of `#[derive(Interface)]`: three implementing object
//! types (`Widget`, `Gadget`, `Gizmo`), a `Node` interface declaring just
//! `id: ID!` — matching a typical real-world `Node` interface shape exactly
//! (`#[graphql(field(name = "id", ty = "ID"))] enum Node {
//! Widget(Widget), .. }`) — and a `node(id: ID!) -> Option<Node>` resolver
//! that dynamically dispatches to whichever variant matches the id. Proves:
//! (a) `__typename` on an interface-typed field resolves to the concrete
//! variant, not the interface name; (b) inline fragments (`... on Widget`)
//! only pull fields from the matching branch; (c) a list of interface values
//! (`Vec<Node>`) resolves each element independently; (d) the SDL prints
//! `interface Node { id: ID! }` plus `type Widget implements Node { .. }` for
//! every variant — no hand-written `OutputType`/`InterfaceType` impl
//! anywhere below.

use flash_graphql::{ID, Interface, Object, Schema, SimpleObject};

#[derive(SimpleObject, Clone)]
struct Widget {
    id: ID,
    name: String,
}

#[derive(SimpleObject, Clone)]
struct Gadget {
    id: ID,
    watts: i32,
}

/// A gizmo — no fields of its own beyond `id`, proving a variant need not
/// add anything past what the interface itself declares.
#[derive(SimpleObject, Clone)]
struct Gizmo {
    id: ID,
}

#[derive(Interface)]
#[graphql(field(name = "id", ty = "ID"))]
enum Node {
    Widget(Widget),
    Gadget(Gadget),
    Gizmo(Gizmo),
}

#[derive(Default)]
struct QueryRoot;

#[Object]
impl QueryRoot {
    async fn node(&self, id: ID) -> Option<Node> {
        if id.0.starts_with("widget:") {
            Some(Node::Widget(Widget {
                id: id.clone(),
                name: "Drill".into(),
            }))
        } else if id.0.starts_with("gadget:") {
            Some(Node::Gadget(Gadget {
                id: id.clone(),
                watts: 42,
            }))
        } else if id.0.starts_with("gizmo:") {
            Some(Node::Gizmo(Gizmo { id: id.clone() }))
        } else {
            None
        }
    }

    async fn all_nodes(&self) -> Vec<Node> {
        vec![
            Node::Widget(Widget {
                id: ID("widget:1".into()),
                name: "Drill".into(),
            }),
            Node::Gadget(Gadget {
                id: ID("gadget:1".into()),
                watts: 7,
            }),
            Node::Gizmo(Gizmo {
                id: ID("gizmo:1".into()),
            }),
        ]
    }
}

#[tokio::test]
async fn interface_dynamic_dispatch_and_sdl() {
    let schema = Schema::build(
        QueryRoot,
        flash_graphql::EmptyMutation,
        flash_graphql::EmptySubscription,
    )
    .finish()
    .expect("schema should build");

    let sdl = schema.sdl();
    println!("--- SDL ---\n{sdl}");
    assert!(sdl.contains("interface Node"), "sdl:\n{sdl}");
    assert!(sdl.contains("type Widget implements Node"), "sdl:\n{sdl}");
    assert!(sdl.contains("type Gadget implements Node"), "sdl:\n{sdl}");
    assert!(sdl.contains("type Gizmo implements Node"), "sdl:\n{sdl}");

    let query = r#"{
        a: node(id: "widget:1") { __typename id ... on Widget { name } }
        b: node(id: "gadget:1") { __typename id ... on Gadget { watts } }
        c: node(id: "gizmo:1") { __typename id }
        d: node(id: "unknown:1") { __typename }
        all: allNodes {
            __typename
            id
            ... on Widget { name }
            ... on Gadget { watts }
        }
    }"#;
    let response = schema.execute(query).await;
    assert!(response.errors.is_empty(), "errors: {:#?}", response.errors);
    let json: serde_json::Value = serde_json::to_value(&response.data).unwrap();
    println!(
        "--- query result ---\n{}",
        serde_json::to_string_pretty(&json).unwrap()
    );

    assert_eq!(json["a"]["__typename"], "Widget");
    assert_eq!(json["a"]["id"], "widget:1");
    assert_eq!(json["a"]["name"], "Drill");

    assert_eq!(json["b"]["__typename"], "Gadget");
    assert_eq!(json["b"]["id"], "gadget:1");
    assert_eq!(json["b"]["watts"], 42);

    assert_eq!(json["c"]["__typename"], "Gizmo");
    assert_eq!(json["c"]["id"], "gizmo:1");

    assert!(
        json["d"].is_null(),
        "unknown id should resolve to null: {json:#?}"
    );

    let all = json["all"].as_array().unwrap();
    assert_eq!(all.len(), 3, "all: {all:#?}");
    assert_eq!(all[0]["__typename"], "Widget");
    assert_eq!(all[0]["name"], "Drill");
    assert_eq!(all[1]["__typename"], "Gadget");
    assert_eq!(all[1]["watts"], 7);
    assert_eq!(all[2]["__typename"], "Gizmo");
}
