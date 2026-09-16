//! End-to-end test of `flash-graphql-derive`: the *same* shape
//! `tests/smoke.rs` hand-writes, built here purely from
//! `#[derive(Enum)]`/`#[derive(SimpleObject)]`/`#[ComplexObject]`/
//! `#[Object]`/`#[derive(InputObject)]`/`#[derive(MergedObject)]` — no
//! hand-written `OutputType`/`InputType`/`RootFields`/`Flatten`/
//! `ComplexObjectFields` impl anywhere below.
//!
//! Covers: an enum with a renamed variant, a plain `SimpleObject`
//! (`Address`), a `SimpleObject` combining `#[graphql(flatten)]`,
//! `#[graphql(skip)]`, a renamed field, and `#[graphql(complex)]` (`Widget`)
//! with a separate `#[ComplexObject]` impl adding a computed field, three
//! `#[Object]` domain query structs (one with an object-level guard, one
//! with both an always-allow object-level guard and an always-deny
//! field-level guard, proving both the deny path and object-before-field
//! ordering), a `#[derive(MergedObject)]` root folding all three together
//! (and proving member types don't leak into the SDL), an `#[Object]`
//! mutation root, and an `InputObject` with a `default` field.

use std::sync::Mutex;

use flash_graphql::{
    ComplexObject, Context, Enum, Error, Guard, ID, InputObject, MergedObject, Object, Result,
    Schema, SimpleObject,
};

// ---------------------------------------------------------------------------
// Guards — one that always allows, one that always denies. Both log to
// `GUARD_LOG` so the test can assert not just pass/fail but *ordering*
// (object-level guard before field-level guard).
// ---------------------------------------------------------------------------

static GUARD_LOG: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

struct Allow;

impl Guard for Allow {
    async fn check(&self, _ctx: &Context<'_>) -> Result<()> {
        GUARD_LOG.lock().unwrap().push("allow");
        Ok(())
    }
}

struct Deny;

impl Guard for Deny {
    async fn check(&self, _ctx: &Context<'_>) -> Result<()> {
        GUARD_LOG.lock().unwrap().push("deny");
        Err(Error::new("access denied by guard"))
    }
}

// ---------------------------------------------------------------------------
// `Priority` — `#[derive(Enum)]` with one renamed variant.
// ---------------------------------------------------------------------------

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
enum Priority {
    Low,
    Medium,
    #[graphql(name = "SEV1")]
    High,
}

// ---------------------------------------------------------------------------
// `Address` — a plain `SimpleObject`, used both directly and flattened.
// ---------------------------------------------------------------------------

/// A postal address.
#[derive(SimpleObject, Debug, Clone)]
struct Address {
    city: String,
    zip: Option<String>,
}

// ---------------------------------------------------------------------------
// `Widget` — renamed field, flattened field, skipped field, `complex`.
// ---------------------------------------------------------------------------

/// A contact phone number, always folded directly onto whatever it's
/// `#[graphql(flatten)]`ed into — proves `flatten` is distinct from a plain
/// nested-object field (`Widget::address` below): `Contact` itself must
/// never appear as its own type in the SDL.
#[derive(SimpleObject, Debug, Clone)]
struct Contact {
    phone: String,
}

/// A thing with a name, a priority, an address, and a contact phone number.
#[derive(SimpleObject, Debug, Clone)]
#[graphql(complex)]
struct Widget {
    id: ID,
    #[graphql(name = "displayName")]
    name: String,
    priority: Priority,
    address: Address,
    #[graphql(flatten)]
    contact: Contact,
    #[graphql(skip)]
    #[allow(dead_code)]
    internal_secret: String,
}

#[ComplexObject]
impl Widget {
    /// The widget's name, shouted.
    async fn shout(&self) -> String {
        self.name.to_uppercase()
    }
}

fn all_widgets() -> Vec<Widget> {
    vec![
        Widget {
            id: ID("1".into()),
            name: "Drill".into(),
            priority: Priority::High,
            address: Address {
                city: "Sydney".into(),
                zip: Some("2000".into()),
            },
            contact: Contact {
                phone: "0400000001".into(),
            },
            internal_secret: "s1".into(),
        },
        Widget {
            id: ID("2".into()),
            name: "Saw".into(),
            priority: Priority::Low,
            address: Address {
                city: "Melbourne".into(),
                zip: None,
            },
            contact: Contact {
                phone: "0400000002".into(),
            },
            internal_secret: "s2".into(),
        },
    ]
}

// ---------------------------------------------------------------------------
// Domain query structs — folded together by `#[derive(MergedObject)]` below.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct WidgetQuery;

#[Object(guard = "Allow")]
impl WidgetQuery {
    async fn widgets(&self, priority: Option<Priority>) -> Vec<Widget> {
        all_widgets()
            .into_iter()
            .filter(|w| priority.is_none_or(|p| p == w.priority))
            .collect()
    }

    async fn widget(&self, #[graphql(desc = "the widget id")] id: ID) -> Option<Widget> {
        all_widgets().into_iter().find(|w| w.id == id)
    }
}

#[derive(Default)]
struct SecretQuery;

#[Object(guard = "Allow")]
impl SecretQuery {
    async fn ping(&self) -> String {
        "pong".into()
    }

    #[graphql(guard = "Deny")]
    async fn secret(&self) -> String {
        "top secret".into()
    }
}

#[derive(Default)]
struct MetaQuery;

#[Object]
impl MetaQuery {
    async fn api_version(&self) -> String {
        "1.0.0".into()
    }

    // Regression test: a method taking `ctx: &Context<'_>` (common in
    // real-world `#[Object]` methods that read `ctx.data::<T>()`), but
    // nothing in this suite exercised a `ctx` parameter until this test was
    // added — the generated call was silently dropping it, undercounting
    // the real method's arity by one. `echo` both takes `ctx` *and* another
    // GraphQL arg after it, so a regression that only fixes the ctx-only
    // case wouldn't catch it.
    async fn echo(&self, ctx: &Context<'_>, suffix: String) -> String {
        format!("{}{suffix}", ctx.data::<String>().unwrap())
    }
}

#[derive(MergedObject)]
struct QueryRoot(WidgetQuery, SecretQuery, MetaQuery);

// ---------------------------------------------------------------------------
// `CreateWidgetInput` — `InputObject` with a `default` field.
// ---------------------------------------------------------------------------

#[derive(InputObject)]
struct CreateWidgetInput {
    name: String,
    #[graphql(default = "Priority::Medium")]
    priority: Priority,
}

#[derive(Default)]
struct MutationRoot;

#[Object]
impl MutationRoot {
    async fn create_widget(&self, input: CreateWidgetInput) -> Widget {
        Widget {
            id: ID("3".into()),
            name: input.name,
            priority: input.priority,
            address: Address {
                city: "Unknown".into(),
                zip: None,
            },
            contact: Contact {
                phone: "unknown".into(),
            },
            internal_secret: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// The test itself.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn builds_and_executes() {
    let schema = Schema::build(
        QueryRoot(WidgetQuery, SecretQuery, MetaQuery),
        MutationRoot,
        flash_graphql::EmptySubscription,
    )
    .data("hello, ".to_string())
    .finish()
    .expect("schema should build");

    let sdl = schema.sdl();
    println!("--- SDL ---\n{sdl}");
    assert!(sdl.contains("enum Priority"), "sdl:\n{sdl}");
    assert!(sdl.contains("SEV1"), "sdl:\n{sdl}");
    assert!(sdl.contains("type Widget"), "sdl:\n{sdl}");
    assert!(sdl.contains("displayName: String!"), "sdl:\n{sdl}");
    assert!(sdl.contains("shout: String!"), "sdl:\n{sdl}");
    assert!(sdl.contains("type Address"), "sdl:\n{sdl}");
    assert!(sdl.contains("phone: String!"), "sdl:\n{sdl}");
    assert!(sdl.contains("input CreateWidgetInput"), "sdl:\n{sdl}");
    assert!(sdl.contains("type QueryRoot"), "sdl:\n{sdl}");
    assert!(sdl.contains("the widget id"), "sdl:\n{sdl}");
    assert!(sdl.contains("A thing with a name"), "sdl:\n{sdl}");
    // Member types folded flat into the merged root must not themselves
    // appear in the SDL's type registry — same for a `flatten`ed field's
    // type.
    assert!(!sdl.contains("type WidgetQuery"), "sdl:\n{sdl}");
    assert!(!sdl.contains("type SecretQuery"), "sdl:\n{sdl}");
    assert!(!sdl.contains("type MetaQuery"), "sdl:\n{sdl}");
    assert!(!sdl.contains("type Contact"), "sdl:\n{sdl}");

    // --- a query that succeeds: merged fields from all 3 domain structs,
    // a flattened field, a renamed field, a computed (`ComplexObject`)
    // field, and an enum argument. ---
    let query = r#"{
        ping
        apiVersion
        echo(suffix: "world")
        widget(id: "1") { displayName priority address { city zip } phone shout }
        widgets(priority: SEV1) { displayName priority }
    }"#;
    let response = schema.execute(query).await;
    assert!(response.errors.is_empty(), "errors: {:#?}", response.errors);
    let json: serde_json::Value = serde_json::to_value(&response.data).unwrap();
    println!(
        "--- query result ---\n{}",
        serde_json::to_string_pretty(&json).unwrap()
    );
    assert_eq!(json["ping"], "pong");
    assert_eq!(json["apiVersion"], "1.0.0");
    assert_eq!(json["echo"], "hello, world");
    assert_eq!(json["widget"]["displayName"], "Drill");
    assert_eq!(json["widget"]["priority"], "SEV1");
    assert_eq!(json["widget"]["address"]["city"], "Sydney");
    assert!(json["widget"]["address"]["zip"].is_string());
    assert_eq!(json["widget"]["phone"], "0400000001");
    assert_eq!(json["widget"]["shout"], "DRILL");
    let widgets = json["widgets"].as_array().unwrap();
    assert_eq!(widgets.len(), 1, "widgets: {widgets:#?}");
    assert_eq!(widgets[0]["displayName"], "Drill");

    // --- a query that trips the guard: object-level `Allow` must run, then
    // field-level `Deny` must run and turn the field into a GraphQL error
    // (not a panic), while the log proves the ordering. ---
    GUARD_LOG.lock().unwrap().clear();
    let guarded = r#"{ secret }"#;
    let response = schema.execute(guarded).await;
    assert!(!response.errors.is_empty(), "expected a guard error");
    assert!(
        response.errors[0].message.contains("access denied"),
        "{:#?}",
        response.errors
    );
    assert_eq!(
        *GUARD_LOG.lock().unwrap(),
        vec!["allow", "deny"],
        "object-level guard must run before field-level guard"
    );

    // --- a mutation exercising `InputObject`'s `default` field. ---
    let mutation = r#"mutation { createWidget(input: { name: "New" }) { displayName priority } }"#;
    let response = schema.execute(mutation).await;
    assert!(response.errors.is_empty(), "errors: {:#?}", response.errors);
    let json: serde_json::Value = serde_json::to_value(&response.data).unwrap();
    assert_eq!(json["createWidget"]["displayName"], "New");
    assert_eq!(json["createWidget"]["priority"], "MEDIUM");
}
