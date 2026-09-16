//! `ComplexObjectFields` — the seam between `#[derive(SimpleObject)]`'s
//! `#[graphql(complex)]` flag and a separate `#[ComplexObject] impl Foo {
//! .. }` block adding computed (possibly async, possibly guarded) fields on
//! top of `Foo`'s plain data fields.
//!
//! When `#[graphql(complex)]` is present, `SimpleObject`'s generated
//! `Flatten::add_flattened_fields`/`register_flattened` call
//! `<Self as ComplexObjectFields>::add_complex_fields`/`register` too, so a
//! complex type keeps its computed fields even when it is itself
//! `#[graphql(flatten)]`ed into another struct (found via a real
//! ORM-entity-backed type that was both `#[graphql(complex)]` *and*
//! flattened into an outer struct — its exported SDL was silently missing
//! the complex fields until this was wired through: the direct,
//! non-flattened path is just the special case `P = Self`, `project =
//! identity`). There is no default/no-op impl
//! here on purpose: a struct marked `complex` with no corresponding
//! `#[ComplexObject]` impl anywhere in the crate should fail to compile with
//! a plain "trait not implemented" error, not silently register zero extra
//! fields.
use crate::Registrar;
use async_graphql::dynamic;

pub trait ComplexObjectFields: Send + Sync + 'static {
    /// Add the computed fields from a `#[ComplexObject]` impl onto `object`
    /// (already carrying the type's own plain fields). `project` recovers
    /// `&Self` from whatever the resolver's actual parent value `P` is —
    /// `Self` itself for a direct registration, or the outer struct for a
    /// flattened one — exactly like `Flatten::add_flattened_fields`'s own
    /// `project` parameter.
    #[must_use]
    fn add_complex_fields<P: 'static>(
        object: dynamic::Object,
        project: impl Fn(&P) -> &Self + Copy + Send + Sync + 'static,
    ) -> dynamic::Object;

    /// Register every type reachable from a computed field's return type
    /// (and argument types, if any).
    fn register(registrar: &mut Registrar);
}
