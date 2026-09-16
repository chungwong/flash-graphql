//! `flash-graphql` — a typed facade in front of `async_graphql::dynamic`.
//!
//! This crate is the core runtime — the
//! [`OutputType`]/[`InputType`] traits, their blanket/scalar impls,
//! [`Registrar`], the [`Schema`]/[`RootFields`] facade, plus [`Flatten`] and
//! [`ComplexObjectFields`] — *and* re-exports the `flash-graphql-derive`
//! proc macros (`Object`, `SimpleObject`, `ComplexObject`, `Enum`,
//! `InputObject`, `MergedObject`) below, so a user writes
//! `use flash_graphql::{Object, SimpleObject, ..};` exactly like
//! async-graphql. `tests/smoke.rs` is the hand-written reference shape the
//! macros mechanically generate; `tests/derive_*.rs` proves the macros
//! themselves produce a working schema with no hand-written trait impls. See
//! this crate's `README.md` for the design rationale.
//!
//! The short version of *why*: `async_graphql`'s code-first derives
//! (`#[Object]`, `SimpleObject`, ...) generate code that gets monomorphized
//! through a generic `ContainerType`/`resolve_container` engine and nests
//! unboxed `async fn` futures inside one another, so touching one resolver's
//! body invalidates and re-codegens everything above it in the object graph
//! — verified to cost ~19s/edit on a real 23K-line schema crate.
//! `async_graphql::dynamic` is type-erased at the boundary
//! (`Field::new(name, TypeRef, move |rc| FieldFuture::Future(Box::pin(async
//! move { .. })))`), so an edit only recompiles that one boxed closure. This
//! crate's traits are the compile-time-checked layer in front of that erased
//! engine: implement [`OutputType`]/[`InputType`] (by hand or via a derive)
//! and get `TypeRef`/registration/`FieldValue` conversion for free, while
//! the user crate itself only ever monomorphizes these small, non-generic
//! trait impls — never the dynamic engine's own machinery.

mod complex;
mod container;
mod input;
mod interface;
mod output;
mod registrar;
mod schema;
mod subscription;

mod scalar;

pub mod connection;
pub mod validators;

pub use complex::ComplexObjectFields;
pub use container::Flatten;
pub use input::InputType;
pub use interface::InterfaceType;
pub use output::OutputType;
pub use registrar::Registrar;
pub use schema::{EmptyMutation, EmptySubscription, RootFields, Schema, SchemaBuilder};
pub use subscription::SubscriptionRoot;

pub use scalar::Json;

/// The proc macros, re-exported with async-graphql's exact names so a real
/// port is a `use` swap: `#[derive(SimpleObject)]`, `#[derive(Enum)]`,
/// `#[derive(InputObject)]`, `#[derive(MergedObject)]`, `#[derive(Interface)]`,
/// `#[derive(OneofObject)]`, the `#[Object]`, `#[ComplexObject]` and
/// `#[Subscription]` attribute macros.
pub use flash_graphql_derive::{
    ComplexObject, Enum, InputObject, Interface, MergedObject, Object, OneofObject, SimpleObject,
    Subscription,
};

/// The pieces of `async_graphql` a user (or a hand-written `OutputType`/
/// `InputType` impl, or a future macro) needs directly: the request context,
/// the crate's error/result types, `ID`/`Value`, and the request/response
/// envelope types `Schema::execute` speaks. Re-exported here rather than
/// requiring a direct `async-graphql` dependency in the user's `Cargo.toml`,
/// so generated code can always say `::flash_graphql::Error` etc. and get
/// the exact version this crate was built against.
pub use async_graphql::{
    Context, Error, ErrorExtensions, Guard, GuardExt, ID, Name, Request, Response, Result, Value,
};

/// Re-exported so macro-generated `InputType::to_value`/`InputObject` code
/// can build `Value::Object` maps as `::flash_graphql::indexmap::IndexMap`
/// without the user's crate needing a direct `async-graphql` dependency.
pub use async_graphql::indexmap;

/// Batch-loading support (`dataloader::{DataLoader, Loader}`), solving
/// the N+1 problem exactly as real async-graphql's own `dataloader` module
/// does — this crate adds nothing on top of it. Dataloader is orthogonal to
/// the static-vs-dynamic schema split this whole crate exists to route
/// around: it only ever touches `ctx.data::<DataLoader<L>>()` (a plain
/// `async_graphql::Context` method, already re-exported above) and
/// `Schema(Builder)::data(..)` (already generic over `D: Any + Send +
/// Sync`), neither of which cares whether the schema itself is code-first
/// static or `dynamic` underneath. Requires the `dataloader` feature (see
/// `Cargo.toml`) — a real-world reference usage is a handful of
/// `impl Loader<K>` blocks consumed via `ctx.data::<DataLoader<L>>()`
/// in `#[ComplexObject]` methods, and `tests/dataloader_smoke.rs` proves
/// real batching (a counter inside a hand-written `Loader` impl, incremented
/// once despite the query fetching several keys).
pub use async_graphql::dataloader;

/// Re-exported so `#[Subscription]`-generated code (see
/// `flash-graphql-derive/src/subscription.rs`) can map a user's plain
/// `Stream<Item = T>` into the `Stream<Item = Result<FieldValue<'static>>>`
/// shape `dynamic::SubscriptionFieldFuture::new` requires, without the
/// user's crate needing a direct `futures-util` dependency — same rationale
/// as `dynamic`/`indexmap` above. `async_graphql` itself already re-exports
/// the exact same `futures_util` crate this crate depends on directly
/// (`lib.rs:235` in `async-graphql-7.0.17`), so a user's own
/// `#[Subscription]` impl can import `futures_util::{Stream, StreamExt,
/// ..}` from this re-export too, with no separate dependency.
pub use async_graphql::futures_util;

/// The full `async_graphql::dynamic` module, re-exported so hand-written
/// `OutputType`/`InputType` impls (and derive-macro-generated code) can
/// reach `Object`, `Field`, `FieldValue`, `FieldFuture`, `TypeRef`,
/// `InputValue`, `Enum`, `EnumItem`, `InputObject`, `Scalar`, `SchemaError`,
/// ... without the user's crate needing its own `async-graphql` dependency.
pub use async_graphql::dynamic;

/// `dynamic::SchemaError` at the crate root too, for convenience (it's the
/// error type `SchemaBuilder::finish` returns).
pub use async_graphql::dynamic::SchemaError;

/// Re-exported for the same reason as `dynamic` above: macro-generated
/// `to_value`/`from_value`-based scalar impls (see `scalar.rs` for the
/// pattern) need these without requiring a direct `async-graphql` dep.
pub use async_graphql::{from_value, to_value};
