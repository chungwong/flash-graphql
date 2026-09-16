//! `InterfaceType` — the compile-time typed counterpart of a GraphQL
//! interface declared with `#[derive(Interface)]` on an enum whose variants
//! each wrap one implementing object type, matching a typical real-world
//! shape such as:
//!
//! ```ignore
//! #[derive(Interface)]
//! #[graphql(field(name = "id", ty = "ID"))]
//! enum Node {
//!     Foo(foo::Foo),
//!     Bar(bar::Bar),
//!     // ...
//! }
//! ```
//!
//! Unlike `OutputType`, which most user types get through `#[derive(...)]`
//! directly, `InterfaceType` exists so `#[derive(Interface)]`'s generated
//! `OutputType` impl has somewhere to (a) register the interface-level
//! fields (`Interface::new(name).field(InterfaceField::new(..))` — a
//! typical `Node` interface declares just `id: ID!`) and (b) ask a *value*
//! which concrete
//! GraphQL object type it should present as, so the generated
//! `resolve_owned`/`resolve_ref` can produce
//! `FieldValue::owned_any(inner).with_type(concrete_name)` — the shape real
//! dynamic-engine `Interface`/`FieldValue::with_type` resolution requires
//! (checked against `async-graphql-7.0.17`'s `dynamic::resolve::resolve_value`:
//! an interface-typed field's value must be `FieldValueInner::WithType { ty,
//! .. }` naming the *implementing* object, not the interface itself, and
//! `ty` is checked at execution time against the interface's registered
//! `possible_types`, which in turn come from each object's own
//! `.implement(interface_name)` — see `Registrar::implement` for how a
//! variant's inner type gets that call before the schema is finished).
use std::borrow::Cow;

use async_graphql::dynamic;

/// See the module docs. Implemented once per `#[derive(Interface)]` enum
/// (never by hand in ordinary user code, though nothing stops it).
pub trait InterfaceType: Send + Sync + 'static {
    /// The bare GraphQL interface name (`"Node"`, ...).
    const NAME: &'static str;

    /// Add this interface's own declared fields (name + type only — no
    /// resolver: interface fields are structural, each implementing object's
    /// own field of the same name is what actually resolves) onto
    /// `interface`. Called once by the generated `OutputType::register`.
    #[must_use]
    fn add_fields(interface: dynamic::Interface) -> dynamic::Interface;

    /// The concrete GraphQL object type name this value should present as
    /// (e.g. `<branch::Branch as OutputType>::type_name()` for a
    /// `Node::Branch(..)` value) — used to build
    /// `FieldValue::{owned,borrowed}_any(inner).with_type(name)`.
    fn type_name(&self) -> Cow<'static, str>;
}
