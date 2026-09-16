//! Collects GraphQL type registrations into an `async_graphql::dynamic`
//! schema builder.
//!
//! `Registrar` is threaded through every [`crate::OutputType::register`],
//! [`crate::InputType::register`] and [`crate::RootFields::register`] call.
//! It exists to fix two things the raw `dynamic::SchemaBuilder` does not
//! handle on its own (checked directly against `async-graphql-7.0.17`'s
//! `dynamic::schema::SchemaBuilder::register`, which just does
//! `self.types.insert(ty.name().to_string(), ty)` into an `IndexMap`):
//!
//! 1. **Dedup.** A type reachable from the schema through more than one path
//!    (a nested object used by two parents, an enum used as both an input and
//!    an output type, ...) must only be built and registered once. [`visit`]
//!    is a `TypeId` set implementations must consult first.
//! 2. **Name collisions.** Two different Rust types both claiming the same
//!    GraphQL type name would otherwise silently overwrite each other in the
//!    `IndexMap` (last registration wins, no error) — a real footgun once
//!    macros are generating these calls. `Registrar` keeps its own
//!    name -> (Rust type) claim table and turns a second, different claim
//!    into a clear error surfaced from [`crate::SchemaBuilder::finish`],
//!    instead of a silent overwrite or an engine panic.
use std::{
    any::{Any, TypeId},
    collections::{HashMap, HashSet},
};

use async_graphql::dynamic;

/// See the module docs.
pub struct Registrar {
    // `Option` only so `dynamic::SchemaBuilder`'s consuming (`fn(self) -> Self`)
    // methods can be called via `take()` / put back; always `Some` between
    // calls into this type.
    builder: Option<dynamic::SchemaBuilder>,
    visited: HashSet<TypeId>,
    claimed: HashMap<String, Claim>,
    errors: Vec<String>,
    // (`Interface`/implements-patching — see `crate::interface`): a
    // `dynamic::Object` built by `register_object` is *not* handed to the
    // underlying `dynamic::SchemaBuilder` immediately — it is staged here so
    // `implement` can still attach `.implement(interface_name)` to it
    // regardless of whether the owning type's `register_object` call or the
    // interface's `implement::<T>()` call happens first (both a
    // `#[derive(Interface)]`'s `OutputType::register` calling
    // `registrar.implement::<Foo>("Node")` *before* delegating to
    // `Foo::register(registrar)`, and the reverse order some other
    // reachability path might produce, both need to work — see the module
    // docs on why raw `dynamic::SchemaBuilder::register` alone can't do
    // this). Patches are applied once, in `into_parts`, just before hand-off.
    pending_objects: Vec<(TypeId, dynamic::Object)>,
    implements: HashMap<TypeId, ImplementClaim>,
}

struct Claim {
    type_id: TypeId,
    rust_type_name: &'static str,
}

/// A pending "this Rust type's GraphQL object must additionally
/// `.implement(interface_name)`" request recorded by [`Registrar::implement`].
struct ImplementClaim {
    interface_names: Vec<&'static str>,
    rust_type_name: &'static str,
}

impl Registrar {
    pub(crate) fn new(builder: dynamic::SchemaBuilder) -> Self {
        Self {
            builder: Some(builder),
            visited: HashSet::new(),
            claimed: HashMap::new(),
            errors: Vec::new(),
            pending_objects: Vec::new(),
            implements: HashMap::new(),
        }
    }

    /// Returns `true` the first time it is called for `T`, `false` on every
    /// later call. `OutputType`/`InputType::register` implementations must
    /// call this first and return immediately on `false` — that's what makes
    /// registering the same type through two paths a no-op instead of a
    /// double registration, and what makes cyclic type graphs terminate.
    pub fn visit<T: 'static>(&mut self) -> bool {
        self.visited.insert(TypeId::of::<T>())
    }

    /// Register a GraphQL object type owned by Rust type `T`. Unlike the
    /// other `register_*` methods, `object` is not handed to the
    /// underlying `dynamic::SchemaBuilder` immediately — it is staged so a
    /// (possibly later) [`Registrar::implement`] call can still patch it with
    /// `.implement(interface_name)` before [`Registrar::into_parts`] hands
    /// everything to the builder. See the module docs and this struct's
    /// `pending_objects` field doc.
    pub fn register_object<T: 'static>(&mut self, object: dynamic::Object) {
        let name = object.type_name().to_string();
        if self.claim::<T>(name) {
            self.pending_objects.push((TypeId::of::<T>(), object));
        }
    }

    /// Register a GraphQL input object type owned by Rust type `T`.
    pub fn register_input_object<T: 'static>(&mut self, input: dynamic::InputObject) {
        self.register_named::<T>(input.type_name().to_string(), input);
    }

    /// Register a GraphQL enum type owned by Rust type `T`.
    pub fn register_enum<T: 'static>(&mut self, e: dynamic::Enum) {
        self.register_named::<T>(e.type_name().to_string(), e);
    }

    /// Register a GraphQL scalar type owned by Rust type `T`.
    pub fn register_scalar<T: 'static>(&mut self, scalar: dynamic::Scalar) {
        self.register_named::<T>(scalar.type_name().to_string(), scalar);
    }

    /// Register a GraphQL scalar that may legitimately be produced by more
    /// than one distinct Rust type sharing one GraphQL name — e.g.
    /// `chrono::DateTime<Utc>` and `chrono::DateTime<Local>` both print as
    /// the `DateTime` scalar (real async-graphql registers a separate
    /// `ScalarType` impl per `Tz` under the same name — see
    /// `async-graphql-7.0.17`'s `types/external/datetime.rs`), and `Json<T>`
    /// for two different `T`s both print as `JSON`. Unlike
    /// [`Registrar::register_scalar`] (keyed by the calling Rust type `T`,
    /// so a second distinct `T` claiming the same name is treated as a real
    /// collision — correct for object/enum/input-object types, which really
    /// must be 1:1 with a GraphQL name), this is keyed by `Key`, a marker
    /// type the scalar's own module picks and every producer of that scalar
    /// shares — so the underlying `dynamic::Scalar` is built and the name
    /// claimed exactly once, and every later call (any producing Rust type)
    /// is a harmless no-op instead of a "claimed by both" error.
    pub fn register_shared_scalar<Key: 'static>(&mut self, scalar: dynamic::Scalar) {
        if !self.visit::<Key>() {
            return;
        }
        self.register_named::<Key>(scalar.type_name().to_string(), scalar);
    }

    /// Register a GraphQL interface type owned by Rust type `T` (the enum a
    /// `#[derive(Interface)]` was applied to). Registered immediately, like
    /// enums/scalars/input objects — only *implementors* (plain objects) need
    /// the deferred `pending_objects` treatment, since the interface itself
    /// never appears in anyone's `implements` list.
    pub fn register_interface<T: 'static>(&mut self, interface: dynamic::Interface) {
        self.register_named::<T>(interface.type_name().to_string(), interface);
    }

    /// Register a GraphQL subscription root type owned by Rust type `T` (the
    /// marker struct a `#[Subscription]` impl was applied to — see
    /// `crate::subscription`). Registered immediately, like
    /// enums/scalars/input objects/interfaces: unlike `register_object`, a
    /// subscription root is never an `implement()` target (a `Subscription`
    /// type can't implement a GraphQL interface), so it needs none of
    /// `register_object`'s deferred `pending_objects` treatment.
    pub fn register_subscription<T: 'static>(&mut self, subscription: dynamic::Subscription) {
        self.register_named::<T>(subscription.type_name().to_string(), subscription);
    }

    /// Record that the GraphQL object type owned by Rust type `T` must
    /// additionally declare `.implement(interface_name)` — the
    /// "implements-patching" a `#[derive(Interface)]`-generated
    /// `OutputType::register` performs on each of its variants' inner types
    /// before (or after — order does not matter, see `pending_objects`)
    /// delegating to that inner type's own `register`.
    pub fn implement<T: 'static>(&mut self, interface_name: &'static str) {
        self.implements
            .entry(TypeId::of::<T>())
            .or_insert_with(|| ImplementClaim {
                interface_names: Vec::new(),
                rust_type_name: std::any::type_name::<T>(),
            })
            .interface_names
            .push(interface_name);
    }

    /// Shared by `register_object`/`register_named`: records `name` as
    /// claimed by `T`, recording a collision error instead if a *different*
    /// Rust type already claimed it. Returns whether the caller should go on
    /// to actually register its type (`false` on a collision, or — harmlessly
    /// — when `T` itself already claimed this exact name, which should not
    /// happen if the caller used `visit` correctly but is not itself a
    /// collision).
    fn claim<T: 'static>(&mut self, name: String) -> bool {
        let type_id = TypeId::of::<T>();
        match self.claimed.get(&name) {
            Some(claim) if claim.type_id == type_id => false,
            Some(claim) => {
                self.errors.push(format!(
                    "GraphQL type name \"{name}\" is claimed by both `{}` and `{}` — \
                     two different Rust types cannot register the same GraphQL type name",
                    claim.rust_type_name,
                    std::any::type_name::<T>(),
                ));
                false
            }
            None => {
                self.claimed.insert(
                    name,
                    Claim {
                        type_id,
                        rust_type_name: std::any::type_name::<T>(),
                    },
                );
                true
            }
        }
    }

    fn register_named<T: 'static>(&mut self, name: String, ty: impl Into<dynamic::Type>) {
        if self.claim::<T>(name) {
            self.with_builder(|b| b.register(ty));
        }
    }

    /// Add global data, forwarded to `dynamic::SchemaBuilder::data`.
    pub(crate) fn data<D: Any + Send + Sync>(&mut self, data: D) {
        self.with_builder(|b| b.data(data));
    }

    fn with_builder(&mut self, f: impl FnOnce(dynamic::SchemaBuilder) -> dynamic::SchemaBuilder) {
        let builder = self.builder.take().expect("Registrar: builder taken twice");
        self.builder = Some(f(builder));
    }

    /// Consumes the registrar, returning the underlying `dynamic` builder and
    /// any errors recorded along the way (name collisions, and a stray
    /// `implement::<T>()` call whose `T` was never actually registered as a
    /// GraphQL object — a real footgun once macros generate these calls, same
    /// rationale as the name-collision check). Applies every pending
    /// `implement` patch to its matching staged object first. Errors take
    /// priority: `SchemaBuilder::finish` checks these before ever calling the
    /// inner `dynamic::SchemaBuilder::finish`.
    pub(crate) fn into_parts(mut self) -> (dynamic::SchemaBuilder, Vec<String>) {
        let pending_objects = std::mem::take(&mut self.pending_objects);
        for (type_id, object) in pending_objects {
            let object = match self.implements.remove(&type_id) {
                Some(claim) => claim
                    .interface_names
                    .into_iter()
                    .fold(object, |object, name| object.implement(name)),
                None => object,
            };
            self.with_builder(|b| b.register(object));
        }
        for claim in self.implements.into_values() {
            self.errors.push(format!(
                "`{}` was marked as implementing interface(s) {:?} but was never registered as a \
                 GraphQL object type",
                claim.rust_type_name, claim.interface_names,
            ));
        }
        (
            self.builder.expect("Registrar: builder missing"),
            self.errors,
        )
    }
}
