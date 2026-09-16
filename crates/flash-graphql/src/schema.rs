use std::{any::Any, marker::PhantomData, sync::Arc};

use async_graphql::{
    Data, Request, Response,
    dynamic::{self, SchemaError},
};
use futures_util::stream::BoxStream;

use crate::{Registrar, SubscriptionRoot};

// Just a readability alias for clippy's `type_complexity` lint — `Q`/`M`/`S`
// never actually exist at runtime, this only pins the three type parameters.
type RootMarker<Q, M, S> = PhantomData<fn() -> (Q, M, S)>;

/// Implemented by the marker types passed to [`Schema::build`] as `Q`/`M`/`S`
/// — typically a zero-sized struct per root (`struct QueryRoot;`), matching
/// async-graphql's own root-type shape. Unlike a `#[derive(MergedObject)]`
/// root value, `add_fields`/`register` never see `&self`: every field
/// resolver gets its data from `ctx.data::<T>()`, never from the root value
/// itself, so the marker carries no state and these are plain associated
/// functions — the instances [`Schema::build`] takes exist only to pin down
/// `Q`/`M`/`S` through type inference at the call site.
pub trait RootFields: Send + Sync + 'static {
    /// The GraphQL name of this root object (`"Query"`, a custom name like
    /// `"QueryRoot"`, ...).
    const NAME: &'static str;

    /// `true` for [`EmptyMutation`]/[`EmptySubscription`] — tells
    /// [`Schema::build`] to skip building/registering an object for this
    /// root entirely and to pass `None` for its type name to the underlying
    /// `dynamic::Schema::build` (which takes `mutation`/`subscription` as
    /// `Option<&str>`).
    const IS_EMPTY: bool = false;

    /// Add this root's fields onto `object` (already constructed and named
    /// `Self::NAME`).
    #[must_use]
    fn add_fields(object: dynamic::Object) -> dynamic::Object {
        object
    }

    /// Register every type reachable from this root's fields (return types,
    /// argument types) into `registrar`.
    fn register(_registrar: &mut Registrar) {}
}

/// A root with no fields — pass as `M` (or `S`) to [`Schema::build`] when
/// there is no mutation (or subscription).
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyMutation;

impl RootFields for EmptyMutation {
    const NAME: &'static str = "Mutation";
    const IS_EMPTY: bool = true;
}

/// A root with no fields — pass as `S` to [`Schema::build`] when there is no
/// subscription.
///
/// Real subscription roots are built via [`crate::SubscriptionRoot`]
/// (`#[Subscription]`, `SubscriptionField`/`SubscriptionFieldFuture` — a
/// genuinely different `dynamic` builder type from `Object`, not a
/// `RootFields` impl with content) rather than this trait — see that
/// module's doc comment. `EmptySubscription`'s own `SubscriptionRoot` impl
/// lives there (next to the trait it implements) rather than here.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptySubscription;

/// Typed facade over a `dynamic::Schema`. `Q`/`M`/`S` are phantom: they exist
/// so the compiler ties this schema to the `RootFields` impls that built it
/// (and so e.g. `async-graphql-axum` extractors see a distinct type per
/// schema, matching real async-graphql's `Schema<Q, M, S>` ergonomics) — no
/// data from `Q`/`M`/`S` is stored at runtime, the whole schema is the one
/// concrete `dynamic::Schema` underneath.
pub struct Schema<Q, M = EmptyMutation, S = EmptySubscription> {
    inner: dynamic::Schema,
    _marker: RootMarker<Q, M, S>,
}

impl<Q, M, S> Clone for Schema<Q, M, S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _marker: PhantomData,
        }
    }
}

impl<Q: RootFields, M: RootFields, S: SubscriptionRoot> Schema<Q, M, S> {
    /// Start building a schema. `query`/`mutation`/`subscription` are only
    /// used to pin down `Q`/`M`/`S` via type inference at the call site —
    /// pass zero-sized marker values (e.g. `QueryRoot`, `EmptyMutation`,
    /// `EmptySubscription`), matching async-graphql's own
    /// `Schema::build(QueryRoot, MutationRoot, EmptySubscription)` call
    /// shape so a real port is a `use` swap.
    pub fn build(_query: Q, _mutation: M, _subscription: S) -> SchemaBuilder<Q, M, S> {
        let mutation_name = (!M::IS_EMPTY).then_some(M::NAME);
        let subscription_name = (!S::IS_EMPTY).then_some(S::NAME);

        let mut registrar = Registrar::new(dynamic::Schema::build(
            Q::NAME,
            mutation_name,
            subscription_name,
        ));

        let query_obj = Q::add_fields(dynamic::Object::new(Q::NAME));
        Q::register(&mut registrar);
        registrar.register_object::<Q>(query_obj);

        if !M::IS_EMPTY {
            let mutation_obj = M::add_fields(dynamic::Object::new(M::NAME));
            M::register(&mut registrar);
            registrar.register_object::<M>(mutation_obj);
        }

        if !S::IS_EMPTY {
            // Unlike `Q`/`M` above, a subscription root builds a
            // `dynamic::Subscription` (a distinct dynamic-engine type from
            // `dynamic::Object`) — see `subscription.rs`'s module doc.
            let subscription_obj = S::add_fields(dynamic::Subscription::new(S::NAME));
            S::register(&mut registrar);
            registrar.register_subscription::<S>(subscription_obj);
        }

        SchemaBuilder {
            registrar,
            _marker: PhantomData,
        }
    }

    /// Returns the Schema Definition Language (SDL) representation of this
    /// schema.
    pub fn sdl(&self) -> String {
        self.inner.sdl()
    }

    /// Execute a GraphQL query/mutation.
    pub async fn execute(&self, request: impl Into<Request>) -> Response {
        self.inner.execute(request.into()).await
    }
}

/// Builder returned from [`Schema::build`]. Mirrors async-graphql's own
/// `SchemaBuilder`: `.data(x)` to seed global data, `.finish()` to validate
/// and get a [`Schema`].
pub struct SchemaBuilder<Q, M, S> {
    registrar: Registrar,
    _marker: RootMarker<Q, M, S>,
}

impl<Q: RootFields, M: RootFields, S: SubscriptionRoot> SchemaBuilder<Q, M, S> {
    /// Add global data, retrievable in a resolver via `ctx.data::<D>()`.
    #[must_use]
    pub fn data<D: Any + Send + Sync>(mut self, data: D) -> Self {
        self.registrar.data(data);
        self
    }

    /// Validate and build the schema. Matches real async-graphql's dynamic
    /// `SchemaBuilder::finish(self) -> Result<Schema, SchemaError>` (7.0.17
    /// does *not* panic here — neither does this facade): a name collision
    /// recorded by the `Registrar` (two different Rust types claiming one
    /// GraphQL type name) is reported the same way any other schema build
    /// error is, before the underlying `dynamic` engine's own `finish` (and
    /// its `check()` pass) even runs.
    pub fn finish(self) -> Result<Schema<Q, M, S>, SchemaError> {
        let (builder, errors) = self.registrar.into_parts();
        if !errors.is_empty() {
            return Err(SchemaError(errors.join("; ")));
        }
        let inner = builder.finish()?;
        Ok(Schema {
            inner,
            _marker: PhantomData,
        })
    }
}

impl<Q: RootFields, M: RootFields, S: SubscriptionRoot> async_graphql::Executor
    for Schema<Q, M, S>
{
    async fn execute(&self, request: Request) -> Response {
        self.inner.execute(request).await
    }

    fn execute_stream(
        &self,
        request: Request,
        session_data: Option<Arc<Data>>,
    ) -> BoxStream<'static, Response> {
        use futures_util::StreamExt;
        self.inner
            .execute_stream_with_session_data(request, session_data.unwrap_or_default())
            .boxed()
    }
}
