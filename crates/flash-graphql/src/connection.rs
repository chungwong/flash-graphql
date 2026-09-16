//! Relay-style pagination: `Connection<Cursor, Node, ConnectionFields,
//! EdgeFields>`, `Edge<Cursor, Node, EdgeFields>`, `PageInfo`, `EmptyFields`,
//! and the `query` helper — flash-graphql's counterpart to real
//! async-graphql's `async_graphql::types::connection` module (its 7.0.17
//! source is the reference this was built against: `connection_type.rs`,
//! `edge.rs`, `page_info.rs`, `cursor.rs`, `mod.rs`'s `query`/`query_with`).
//!
//! This is one of the few places in this crate that uses
//! real generics — deliberately: unlike a user schema type (one macro
//! expansion per concrete type, which is what this whole design exists to
//! keep cheap to rebuild), `Connection`/`Edge` live *in this crate*, so they
//! monomorphize once per instantiation as an ordinary dependency, exactly
//! like async-graphql's own generic `Connection`/`Edge` do today. A user
//! crate touching its own resolver bodies never recompiles this file.
//!
//! ## Design deviations from real async-graphql (deliberate)
//!
//! - **No `Name`/`EdgeName`/`NodesField` generic parameters.** Real
//!   async-graphql lets a caller rename the generated `*Connection`/`*Edge`
//!   type or turn off the `nodes` field. A real-world schema this was
//!   ported against never used either knob (every `*Connection` used the
//!   default `{Node}Connection`/`{Node}Edge` naming and always had both
//!   `edges` *and* `nodes`), so those knobs are just left out rather than
//!   carried along unused.
//! - **`CursorType` replaces real async-graphql's identically-named trait**
//!   with a narrower one covering only what that real-world usage actually
//!   instantiated `Cursor` as (`ID`, and this crate's own tests' `String`)
//!   rather than the full real trait's numeric/`DateTime`/`OpaqueCursor`
//!   surface. Crucially: the GraphQL `cursor`/`startCursor`/`endCursor`
//!   fields are *always* the literal `String` scalar, produced by
//!   `CursorType::encode_cursor`, regardless of what `Cursor` the caller
//!   picks — matching real async-graphql exactly (`Edge`'s `#[ComplexObject]
//!   async fn cursor(&self) -> String`, not `Cursor`'s own type) and a
//!   typical schema's `FooEdge.cursor: String!`/`PageInfo`'s `String` cursor
//!   fields. Because of this, `Cursor` itself is never registered as a
//!   GraphQL type and does not need `OutputType`/`InputType` at all (a
//!   deviation from this design's own initial sketch, which bounded it
//!   `InputType + OutputType` — that bound would have produced `cursor:
//!   ID!` for a `Cursor = ID` instantiation, a genuine SDL mismatch against
//!   a real schema's `cursor: String!`, so it was dropped in favor of the
//!   narrower, correct `CursorType` bound).
use std::borrow::Cow;

use async_graphql::{
    Error, Result, Value,
    dynamic::{self, Field, FieldFuture, FieldValue},
};

use crate::{Flatten, OutputType, Registrar};

// ---------------------------------------------------------------------------
// `CursorType` — see the module doc's "design deviations" section.
// ---------------------------------------------------------------------------

/// A value that can serialize to/from the opaque `String` cursor scalar.
/// Implemented here for the two cursor representations a real-world schema
/// actually needs: a plain `String` (this crate's own tests, and any other
/// use case that doesn't need a typed cursor) and `async_graphql::ID`
/// (`GlobalID`-encoded opaque IDs, already strings under the hood).
pub trait CursorType: Send + Sync + Sized + 'static {
    /// Encode this cursor as the `String` GraphQL sees.
    fn encode_cursor(&self) -> String;

    /// Decode a cursor from the raw string an `after`/`before` argument
    /// carried. Both impls below are infallible (a `String` newtype or a
    /// bare `String`), matching real async-graphql's own `ID`/`String`
    /// `CursorType` impls (`type Error = Infallible`).
    fn decode_cursor(s: &str) -> Result<Self>;
}

impl CursorType for String {
    fn encode_cursor(&self) -> String {
        self.clone()
    }

    fn decode_cursor(s: &str) -> Result<Self> {
        Ok(s.to_string())
    }
}

impl CursorType for async_graphql::ID {
    fn encode_cursor(&self) -> String {
        self.0.clone()
    }

    fn decode_cursor(s: &str) -> Result<Self> {
        Ok(async_graphql::ID(s.to_string()))
    }
}

// ---------------------------------------------------------------------------
// `EmptyFields` — the default `ConnectionFields`/`EdgeFields`: contributes no
// extra fields and is never itself registered as a GraphQL type (matching
// real async-graphql's `#[graphql(internal, fake)]` `EmptyFields`).
// ---------------------------------------------------------------------------

/// No additional fields. The default `ConnectionFields`/`EdgeFields` type
/// parameter for [`Connection`]/[`Edge`].
#[derive(Debug, Clone, Copy, Default)]
pub struct EmptyFields;

impl Flatten for EmptyFields {
    fn add_flattened_fields<P: 'static>(
        object: dynamic::Object,
        _project: impl Fn(&P) -> &Self + Copy + Send + Sync + 'static,
    ) -> dynamic::Object {
        object
    }

    fn register_flattened(_registrar: &mut Registrar) {}
}

// ---------------------------------------------------------------------------
// `PageInfo` — a single, non-generic type shared by every `Connection<..>`
// instantiation (`schema.graphql` has exactly one `PageInfo` block; real
// async-graphql's `PageInfo` isn't generic over `Cursor` either — hand-
// written here, not `#[derive(SimpleObject)]`, only because that macro's
// generated code hard-codes the `::flash_graphql` absolute path, which does
// not resolve from *inside* the `flash-graphql` crate itself without a
// `extern crate self as flash_graphql` alias this crate doesn't declare;
// every derive-macro-consuming test lives in `tests/`, a separate crate that
// depends on this one normally, so it never hits this).
// ---------------------------------------------------------------------------

/// Information about pagination in a connection.
#[derive(Debug, Clone)]
pub struct PageInfo {
    /// When paginating backwards, are there more items?
    pub has_previous_page: bool,
    /// When paginating forwards, are there more items?
    pub has_next_page: bool,
    /// When paginating backwards, the cursor to continue.
    pub start_cursor: Option<String>,
    /// When paginating forwards, the cursor to continue.
    pub end_cursor: Option<String>,
}

impl OutputType for PageInfo {
    fn type_name() -> Cow<'static, str> {
        Cow::Borrowed("PageInfo")
    }

    fn register(registrar: &mut Registrar) {
        if !registrar.visit::<Self>() {
            return;
        }
        <bool as OutputType>::register(registrar);
        <Option<String> as OutputType>::register(registrar);

        let object = dynamic::Object::new("PageInfo")
            .description("Information about pagination in a connection")
            .field(
                Field::new("hasPreviousPage", <bool as OutputType>::type_ref(), |rc| {
                    FieldFuture::Value({
                        let this = rc.parent_value.try_downcast_ref::<PageInfo>().unwrap();
                        <bool as OutputType>::resolve_ref(&this.has_previous_page).unwrap()
                    })
                })
                .description("When paginating backwards, are there more items?"),
            )
            .field(
                Field::new("hasNextPage", <bool as OutputType>::type_ref(), |rc| {
                    FieldFuture::Value({
                        let this = rc.parent_value.try_downcast_ref::<PageInfo>().unwrap();
                        <bool as OutputType>::resolve_ref(&this.has_next_page).unwrap()
                    })
                })
                .description("When paginating forwards, are there more items?"),
            )
            .field(
                Field::new(
                    "startCursor",
                    <Option<String> as OutputType>::type_ref(),
                    |rc| {
                        FieldFuture::Value({
                            let this = rc.parent_value.try_downcast_ref::<PageInfo>().unwrap();
                            <Option<String> as OutputType>::resolve_ref(&this.start_cursor).unwrap()
                        })
                    },
                )
                .description("When paginating backwards, the cursor to continue."),
            )
            .field(
                Field::new(
                    "endCursor",
                    <Option<String> as OutputType>::type_ref(),
                    |rc| {
                        FieldFuture::Value({
                            let this = rc.parent_value.try_downcast_ref::<PageInfo>().unwrap();
                            <Option<String> as OutputType>::resolve_ref(&this.end_cursor).unwrap()
                        })
                    },
                )
                .description("When paginating forwards, the cursor to continue."),
            );
        registrar.register_object::<Self>(object);
    }

    fn resolve_owned(self) -> Result<Option<FieldValue<'static>>> {
        Ok(Some(FieldValue::owned_any(self)))
    }

    fn resolve_ref(&self) -> Result<Option<FieldValue<'_>>> {
        Ok(Some(FieldValue::borrowed_any(self)))
    }
}

// ---------------------------------------------------------------------------
// `Edge<Cursor, Node, EdgeFields>` — one instantiation per `(Cursor, Node,
// EdgeFields)` triple becomes its own named GraphQL object type (`{Node}Edge`
// by default), exactly like real async-graphql's `DefaultEdgeName`.
// ---------------------------------------------------------------------------

/// An edge in a connection.
pub struct Edge<Cursor, Node, EdgeFields = EmptyFields> {
    /// A cursor for use in pagination. Always rendered as the GraphQL
    /// `String` scalar (`CursorType::encode_cursor`), never as `Cursor`'s own
    /// (nonexistent, in this design) GraphQL type.
    pub cursor: Cursor,
    /// The item at the end of the edge.
    pub node: Node,
    /// Additional fields, flattened directly onto the edge object (see
    /// [`Flatten`]) — `EmptyFields` by default, contributing nothing.
    pub additional_fields: EdgeFields,
}

impl<Cursor, Node> Edge<Cursor, Node, EmptyFields>
where
    Cursor: CursorType,
    Node: OutputType,
{
    /// Create a new edge with no additional fields.
    pub fn new(cursor: Cursor, node: Node) -> Self {
        Self {
            cursor,
            node,
            additional_fields: EmptyFields,
        }
    }
}

impl<Cursor, Node, EdgeFields> Edge<Cursor, Node, EdgeFields>
where
    Cursor: CursorType,
    Node: OutputType,
    EdgeFields: Flatten,
{
    /// Create a new edge, with some additional fields flattened onto it.
    pub fn with_additional_fields(
        cursor: Cursor,
        node: Node,
        additional_fields: EdgeFields,
    ) -> Self {
        Self {
            cursor,
            node,
            additional_fields,
        }
    }
}

impl<Cursor, Node, EdgeFields> OutputType for Edge<Cursor, Node, EdgeFields>
where
    Cursor: CursorType,
    Node: OutputType,
    EdgeFields: Flatten,
{
    fn type_name() -> Cow<'static, str> {
        Cow::Owned(format!("{}Edge", Node::type_name()))
    }

    fn register(registrar: &mut Registrar) {
        if !registrar.visit::<Self>() {
            return;
        }
        Node::register(registrar);
        EdgeFields::register_flattened(registrar);

        let name = Self::type_name().into_owned();
        let object = dynamic::Object::new(name)
            .description("An edge in a connection.")
            .field(
                Field::new("node", Node::type_ref(), |rc| {
                    FieldFuture::Value({
                        let this = rc.parent_value.try_downcast_ref::<Self>().unwrap();
                        Node::resolve_ref(&this.node).unwrap()
                    })
                })
                .description("The item at the end of the edge"),
            )
            .field(
                Field::new("cursor", <String as OutputType>::type_ref(), |rc| {
                    FieldFuture::Value({
                        let this = rc.parent_value.try_downcast_ref::<Self>().unwrap();
                        Some(FieldValue::value(Value::String(
                            this.cursor.encode_cursor(),
                        )))
                    })
                })
                .description("A cursor for use in pagination"),
            );
        let object =
            EdgeFields::add_flattened_fields::<Self>(object, |s: &Self| &s.additional_fields);
        registrar.register_object::<Self>(object);
    }

    fn resolve_owned(self) -> Result<Option<FieldValue<'static>>> {
        Ok(Some(FieldValue::owned_any(self)))
    }

    fn resolve_ref(&self) -> Result<Option<FieldValue<'_>>> {
        Ok(Some(FieldValue::borrowed_any(self)))
    }
}

// ---------------------------------------------------------------------------
// `Connection<Cursor, Node, ConnectionFields, EdgeFields>` — one instantiation
// per `(Cursor, Node, ConnectionFields, EdgeFields)` tuple becomes its own
// named GraphQL object type (`{Node}Connection` by default).
// ---------------------------------------------------------------------------

/// The result of a paginated query. Build with [`Connection::new`] (no
/// additional fields) or [`Connection::with_additional_fields`], push onto
/// `.edges`, and return it (wrapped in `Result`/`Option` as needed) from a
/// resolver — or, more commonly, build one with [`query`].
pub struct Connection<Cursor, Node, ConnectionFields = EmptyFields, EdgeFields = EmptyFields> {
    /// All edges of the current page.
    pub edges: Vec<Edge<Cursor, Node, EdgeFields>>,
    /// Additional fields, flattened directly onto the connection object.
    pub additional_fields: ConnectionFields,
    /// If `true`, there is a previous page.
    pub has_previous_page: bool,
    /// If `true`, there is a next page.
    pub has_next_page: bool,
}

impl<Cursor, Node, EdgeFields> Connection<Cursor, Node, EmptyFields, EdgeFields>
where
    Cursor: CursorType,
    Node: OutputType,
    EdgeFields: Flatten,
{
    /// Create a new connection, with no additional fields.
    pub fn new(has_previous_page: bool, has_next_page: bool) -> Self {
        Self {
            edges: Vec::new(),
            additional_fields: EmptyFields,
            has_previous_page,
            has_next_page,
        }
    }
}

impl<Cursor, Node, ConnectionFields, EdgeFields>
    Connection<Cursor, Node, ConnectionFields, EdgeFields>
where
    Cursor: CursorType,
    Node: OutputType,
    ConnectionFields: Flatten,
    EdgeFields: Flatten,
{
    /// Create a new connection, with some additional fields flattened onto
    /// it (a `struct ConnectionTotal { total: i64 }`, e.g.).
    pub fn with_additional_fields(
        has_previous_page: bool,
        has_next_page: bool,
        additional_fields: ConnectionFields,
    ) -> Self {
        Self {
            edges: Vec::new(),
            additional_fields,
            has_previous_page,
            has_next_page,
        }
    }

    fn page_info(&self) -> PageInfo {
        PageInfo {
            has_previous_page: self.has_previous_page,
            has_next_page: self.has_next_page,
            start_cursor: self.edges.first().map(|e| e.cursor.encode_cursor()),
            end_cursor: self.edges.last().map(|e| e.cursor.encode_cursor()),
        }
    }
}

impl<Cursor, Node, ConnectionFields, EdgeFields> OutputType
    for Connection<Cursor, Node, ConnectionFields, EdgeFields>
where
    Cursor: CursorType,
    Node: OutputType,
    ConnectionFields: Flatten,
    EdgeFields: Flatten,
{
    fn type_name() -> Cow<'static, str> {
        Cow::Owned(format!("{}Connection", Node::type_name()))
    }

    fn register(registrar: &mut Registrar) {
        if !registrar.visit::<Self>() {
            return;
        }
        <Edge<Cursor, Node, EdgeFields> as OutputType>::register(registrar);
        <PageInfo as OutputType>::register(registrar);
        <Node as OutputType>::register(registrar);
        ConnectionFields::register_flattened(registrar);

        let name = Self::type_name().into_owned();
        let object = dynamic::Object::new(name)
            .field(
                Field::new("pageInfo", <PageInfo as OutputType>::type_ref(), |rc| {
                    FieldFuture::Value({
                        let this = rc.parent_value.try_downcast_ref::<Self>().unwrap();
                        Some(FieldValue::owned_any(this.page_info()))
                    })
                })
                .description("Information to aid in pagination."),
            )
            .field(
                Field::new(
                    "edges",
                    <Vec<Edge<Cursor, Node, EdgeFields>> as OutputType>::type_ref(),
                    |rc| {
                        FieldFuture::Value({
                            let this = rc.parent_value.try_downcast_ref::<Self>().unwrap();
                            <Vec<Edge<Cursor, Node, EdgeFields>> as OutputType>::resolve_ref(
                                &this.edges,
                            )
                            .unwrap()
                        })
                    },
                )
                .description("A list of edges."),
            )
            .field(
                Field::new("nodes", <Vec<Node> as OutputType>::type_ref(), |rc| {
                    FieldFuture::Value({
                        let this = rc.parent_value.try_downcast_ref::<Self>().unwrap();
                        let items = this.edges.iter().map(|e| {
                            Node::resolve_ref(&e.node)
                                .unwrap()
                                .unwrap_or(FieldValue::NULL)
                        });
                        Some(FieldValue::list(items))
                    })
                })
                .description("A list of nodes."),
            );
        let object =
            ConnectionFields::add_flattened_fields::<Self>(object, |s: &Self| &s.additional_fields);
        registrar.register_object::<Self>(object);
    }

    fn resolve_owned(self) -> Result<Option<FieldValue<'static>>> {
        Ok(Some(FieldValue::owned_any(self)))
    }

    fn resolve_ref(&self) -> Result<Option<FieldValue<'_>>> {
        Ok(Some(FieldValue::borrowed_any(self)))
    }
}

// ---------------------------------------------------------------------------
// `query` — parses/validates `after`/`before`/`first`/`last`, then hands
// decoded values to the callback. Mirrors real async-graphql's
// `connection::query`/`query_with` (`types/connection/mod.rs`) closely
// enough that a real-world caller's own connection-fetching helper needs
// only a mechanical `async_graphql::connection` -> `flash_graphql::connection`
// import swap.
// ---------------------------------------------------------------------------

/// Parse `first`/`last` (non-negative) and decode `after`/`before` (via
/// [`CursorType::decode_cursor`]), then invoke `f` with the decoded values
/// and return its `Connection`. Same validation as real async-graphql's
/// `query_with`: negative `first`/`last` is a hard error here; "exactly one
/// of `first`/`last` must be supplied" and any max-page-size clamping are
/// deliberately *not* enforced here (real async-graphql doesn't either — that
/// validation lives in the caller's own callback, e.g. an
/// `offset_and_limit_for_query`-style helper).
pub async fn query<Cursor, Node, ConnectionFields, EdgeFields, F, Fut, E>(
    after: Option<String>,
    before: Option<String>,
    first: Option<i32>,
    last: Option<i32>,
    f: F,
) -> Result<Connection<Cursor, Node, ConnectionFields, EdgeFields>>
where
    Cursor: CursorType,
    Node: OutputType,
    ConnectionFields: Flatten,
    EdgeFields: Flatten,
    F: FnOnce(Option<Cursor>, Option<Cursor>, Option<usize>, Option<usize>) -> Fut,
    Fut: std::future::Future<
            Output = std::result::Result<Connection<Cursor, Node, ConnectionFields, EdgeFields>, E>,
        >,
    E: Into<Error>,
{
    let first = match first {
        Some(first) if first < 0 => {
            return Err(Error::new(
                "The \"first\" parameter must be a non-negative number",
            ));
        }
        Some(first) => Some(first as usize),
        None => None,
    };
    let last = match last {
        Some(last) if last < 0 => {
            return Err(Error::new(
                "The \"last\" parameter must be a non-negative number",
            ));
        }
        Some(last) => Some(last as usize),
        None => None,
    };
    let before = match before {
        Some(before) => Some(Cursor::decode_cursor(&before)?),
        None => None,
    };
    let after = match after {
        Some(after) => Some(Cursor::decode_cursor(&after)?),
        None => None,
    };
    f(after, before, first, last).await.map_err(Into::into)
}
