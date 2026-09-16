//! Support for `#[derive(SimpleObject)]`'s own-field emission and
//! `#[graphql(flatten)]`.
//!
//! A flattened field's own fields must be folded directly into the *outer*
//! object's field set (never registered as their own GraphQL type) — e.g.
//! `struct Widget { #[graphql(flatten)] address: Address, .. }` makes
//! `Address`'s fields (`city`, `zip`, ...) appear directly on `Widget` in the
//! SDL, not nested under an `address` field. Since a proc-macro expanding
//! `#[derive(SimpleObject)] struct Widget` cannot see `Address`'s own field
//! list (that's a separate, independently-expanded derive invocation), the
//! two derives must communicate through a shared runtime mechanism instead:
//! this trait. Every `#[derive(SimpleObject)]` type implements it (so it
//! doubles as "add my own fields", called with an identity projection by the
//! type's own [`crate::OutputType::register`]); a `flatten` field just
//! recurses into the inner type's [`Flatten::add_flattened_fields`] with a
//! composed projection instead of adding one `Field` for the field itself.
//!
//! This is the one place this crate's macros use a small generic method
//! (rather than the fully non-generic style the rest of the design favors,
//! per the plan's "Why async-graphql is slow to rebuild") — but it is
//! monomorphized only once per distinct outer container that actually uses
//! `flatten`, not per schema type, so it does not reintroduce the nested
//! generic-engine cost this crate exists to avoid.
use crate::Registrar;
use async_graphql::dynamic;

pub trait Flatten: Send + Sync + 'static {
    /// Add this type's own (non-`skip`) fields onto `object`, projecting
    /// through `project` to reach `&Self` from whatever the ultimate outer
    /// container type `P` is (`P = Self` for a type's own registration; some
    /// ancestor type when reached through one or more `flatten` hops).
    #[must_use]
    fn add_flattened_fields<P: 'static>(
        object: dynamic::Object,
        project: impl Fn(&P) -> &Self + Copy + Send + Sync + 'static,
    ) -> dynamic::Object;

    /// Register every type reachable from this type's own fields (recursing
    /// into a `flatten` field's own `register_flattened` rather than
    /// registering the flattened field's type itself as a GraphQL type).
    fn register_flattened(registrar: &mut Registrar);
}
