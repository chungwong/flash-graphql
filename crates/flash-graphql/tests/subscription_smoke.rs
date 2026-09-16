//! End-to-end test of `#[Subscription]`, matching a typical real-world
//! subscription shape (`#[Subscription] impl
//! Subscription { #[graphql(guard = "..")] async fn field(&self, ctx: ..) ->
//! Result<impl Stream<Item = T>> { .. } }`).
//!
//! Proves: (a) a subscription field's stream really drives through the real
//! dynamic engine's `execute_stream` — `dynamic::Schema::execute_stream`,
//! reached here through `flash_graphql::Schema`'s `Executor` impl —
//! yielding one `Response` per stream item, each carrying the right value,
//! in order; (b) a guarded subscription field rejects before the stream
//! ever starts, producing a single error `Response` (real async-graphql's
//! dynamic subscription resolution short-circuits the whole stream on a
//! resolver error — see `dynamic/subscription.rs`'s `collect_streams`), not
//! a panic or a silently-empty stream.

use async_graphql::{Executor, Request};
use flash_graphql::{Context, EmptyMutation, Guard, Object, Result, Schema, Subscription};
use futures_util::{Stream, StreamExt};

#[derive(Default)]
struct QueryRoot;

#[Object]
impl QueryRoot {
    async fn ping(&self) -> String {
        "pong".into()
    }
}

/// Stands in for a typical real-world auth guard — always denies.
struct DenyAll;

impl Guard for DenyAll {
    async fn check(&self, _ctx: &Context<'_>) -> Result<()> {
        Err(flash_graphql::Error::new("denied"))
    }
}

#[derive(Default)]
struct SubscriptionRoot;

#[Subscription]
impl SubscriptionRoot {
    /// Matches a typical real-world subscription method shape: `async
    /// fn(&self, ..) -> Result<impl Stream<Item = T>>`, a plain
    /// `futures_util::stream::iter` standing in for a real pub/sub-backed
    /// chain (out of scope — this proves the mechanism, not any particular
    /// broker).
    async fn counter(&self) -> Result<impl Stream<Item = i32>> {
        Ok(futures_util::stream::iter(vec![1, 2, 3]))
    }

    #[graphql(guard = "DenyAll")]
    async fn secret(&self) -> Result<impl Stream<Item = i32>> {
        Ok(futures_util::stream::iter(vec![42]))
    }
}

fn build_schema() -> Schema<QueryRoot, EmptyMutation, SubscriptionRoot> {
    Schema::build(QueryRoot, EmptyMutation, SubscriptionRoot)
        .finish()
        .expect("schema should build")
}

#[tokio::test]
async fn sdl_shows_subscription_root() {
    let schema = build_schema();
    let sdl = schema.sdl();
    println!("--- SDL ---\n{sdl}");
    assert!(sdl.contains("type Subscription"), "sdl:\n{sdl}");
    assert!(sdl.contains("counter: Int!"), "sdl:\n{sdl}");
}

#[tokio::test]
async fn subscription_stream_yields_every_item_in_order() {
    let schema = build_schema();
    let mut stream = schema.execute_stream(Request::new("subscription { counter }"), None);
    let mut values = Vec::new();
    while let Some(response) = stream.next().await {
        assert!(response.errors.is_empty(), "errors: {:#?}", response.errors);
        let json: serde_json::Value = serde_json::to_value(&response.data).unwrap();
        values.push(json["counter"].as_i64().unwrap());
    }
    assert_eq!(values, vec![1, 2, 3]);
}

#[tokio::test]
async fn guarded_subscription_field_rejects_before_streaming() {
    let schema = build_schema();
    let mut stream = schema.execute_stream(Request::new("subscription { secret }"), None);
    let response = stream.next().await.expect("expected at least one response");
    assert!(
        !response.errors.is_empty(),
        "expected a guard error, got {response:#?}"
    );
    assert!(
        response.errors[0].message.contains("denied"),
        "{:#?}",
        response.errors
    );
    // The stream ends right after the guard rejection — nothing more is
    // ever yielded (no partial/garbage values sneak through).
    assert!(stream.next().await.is_none());
}
