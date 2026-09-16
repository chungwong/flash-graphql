//! End-to-end test of `async_graphql::dataloader::{DataLoader, Loader}`
//! used against a real `flash_graphql::Schema` — matching a typical
//! real-world `DbLoader` shape (`impl Loader<UserId> for
//! DbLoader { .. }`, consumed via `ctx.data::<DataLoader<DbLoader>>()?
//! .load_one(key).await` inside a `#[ComplexObject]` resolver, once per item
//! in a list of parent objects — a `User::groups`/`grants`-style pattern
//! applied across a `Vec<User>`, the classic N+1 shape `DataLoader` exists
//! to fix).
//!
//! Dataloader is orthogonal to the static-vs-dynamic schema split this
//! crate's traits/macros exist to route around — see `lib.rs`'s doc comment
//! on the `dataloader` re-export — so nothing in `flash-graphql` or
//! `flash-graphql-derive` had to change to make this work; these tests prove
//! that claim rather than exercise any new code path of this crate's own.
//!
//! Two levels of proof, both backed by the same `CountingLoader` (a
//! `HashMap` "database" plus an `Arc<AtomicUsize>` incremented once per real
//! `Loader::load` call — never once per key):
//!
//! - `bare_loader_batches_concurrent_load_one_calls`: no GraphQL/schema
//!   involved at all — 3 concurrent `DataLoader::load_one` calls
//!   (`tokio::join!`) against the same `DataLoader` batch into 1 `load` call.
//!   Isolates "does async-graphql's own `DataLoader` really coalesce" from
//!   any question about this crate's engine/resolver plumbing.
//! - `graphql_list_query_batches_across_items`: a real query against a real
//!   `flash_graphql::Schema`, `{ items { id name } }`, where `items` returns
//!   `Vec<Item>` and each `Item`'s `name` field independently calls
//!   `load_one` from a `#[ComplexObject]` resolver. The real dynamic engine
//!   resolves a field's list value's elements *concurrently*
//!   (`dynamic::resolve::resolve_list`'s `try_join_all` — verified against
//!   `async-graphql-7.0.17`'s source directly), so the 3 items' independent
//!   `load_one` calls reach the loader concurrently and batch into 1 real
//!   `Loader::load` call — proving the mechanism survives contact with this
//!   crate's actual generated `#[ComplexObject]`/`#[Object]` resolver code,
//!   not just bare async-graphql. (Nested single-object field sets — e.g.
//!   `Item`'s own sibling fields — used to be resolved serially pre-patch,
//!   `resolve_container(.., serial=true)`; the vendor patch in
//!   `vendor/PATCH.md` makes those concurrent too, proved separately by
//!   `tests/partial_response_smoke.rs`'s
//!   `sibling_fields_on_one_object_resolve_concurrently`.)

use std::{
    collections::HashMap,
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use flash_graphql::{
    ComplexObject, Context, EmptyMutation, EmptySubscription, Object, Schema, SimpleObject,
    dataloader::{DataLoader, Loader},
};

/// Stands in for a typical real-world `DbLoader`: a batch loader backed by
/// an in-memory "database" instead of a real ORM-backed fetch, plus a call
/// counter a production `DbLoader` itself would have no reason to carry
/// (this test's own addition, to prove batching real async-graphql's
/// `DataLoader` performs under the hood — not something production code
/// needs to assert itself).
struct CountingLoader {
    db: HashMap<i32, String>,
    calls: Arc<AtomicUsize>,
}

impl Loader<i32> for CountingLoader {
    type Value = String;
    type Error = Infallible;

    async fn load(&self, keys: &[i32]) -> Result<HashMap<i32, Self::Value>, Self::Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(keys
            .iter()
            .filter_map(|k| self.db.get(k).map(|v| (*k, v.clone())))
            .collect())
    }
}

fn sample_db() -> HashMap<i32, String> {
    HashMap::from([
        (1, "Alice".to_string()),
        (2, "Bob".to_string()),
        (3, "Carol".to_string()),
    ])
}

#[tokio::test]
async fn bare_loader_batches_concurrent_load_one_calls() {
    let calls = Arc::new(AtomicUsize::new(0));
    let loader = DataLoader::new(
        CountingLoader {
            db: sample_db(),
            calls: calls.clone(),
        },
        tokio::spawn,
    );

    let (a, b, c) = tokio::join!(loader.load_one(1), loader.load_one(2), loader.load_one(3));
    assert_eq!(a.unwrap(), Some("Alice".to_string()));
    assert_eq!(b.unwrap(), Some("Bob".to_string()));
    assert_eq!(c.unwrap(), Some("Carol".to_string()));

    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "expected 3 concurrent `load_one` calls to batch into a single `Loader::load`, got {}",
        calls.load(Ordering::SeqCst)
    );
}

/// A typical `#[graphql(complex)]` pattern: a `SimpleObject`'s plain
/// `id` field plus a computed, loader-backed `name` field.
#[derive(SimpleObject)]
#[graphql(complex)]
struct Item {
    id: i32,
}

#[ComplexObject]
impl Item {
    async fn name(&self, ctx: &Context<'_>) -> Option<String> {
        let loader = ctx.data::<DataLoader<CountingLoader>>().ok()?;
        loader.load_one(self.id).await.ok()?
    }
}

#[derive(Default)]
struct QueryRoot;

#[Object]
impl QueryRoot {
    async fn items(&self) -> Vec<Item> {
        vec![Item { id: 1 }, Item { id: 2 }, Item { id: 3 }]
    }
}

#[tokio::test]
async fn graphql_list_query_batches_across_items() {
    let calls = Arc::new(AtomicUsize::new(0));
    let loader = DataLoader::new(
        CountingLoader {
            db: sample_db(),
            calls: calls.clone(),
        },
        tokio::spawn,
    );

    let schema = Schema::build(QueryRoot, EmptyMutation, EmptySubscription)
        .data(loader)
        .finish()
        .expect("schema should build");

    let response = schema.execute("{ items { id name } }").await;
    assert!(response.errors.is_empty(), "errors: {:#?}", response.errors);

    let json: serde_json::Value = serde_json::to_value(&response.data).unwrap();
    assert_eq!(json["items"][0]["name"], "Alice");
    assert_eq!(json["items"][1]["name"], "Bob");
    assert_eq!(json["items"][2]["name"], "Carol");

    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "expected the 3 list items' independent `load_one` calls to batch into a single \
         `Loader::load`, got {} calls",
        calls.load(Ordering::SeqCst)
    );
}
