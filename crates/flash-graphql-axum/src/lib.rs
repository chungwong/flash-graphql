//! `flash-graphql-axum` — real `async-graphql-axum`, completely unmodified
//! (`pub use async_graphql_axum::*` below), made to work against a
//! `flash_graphql::Schema<Q, M, S>` by nothing more than that type's own
//! `Executor` impl.
//!
//! `async-graphql-axum`'s extractors/handlers (`GraphQLRequest`,
//! `GraphQLResponse`, `GraphQLWebSocket`, `GraphQLProtocol`) are generic over
//! `E: async_graphql::Executor` (checked directly against
//! `async-graphql-axum-7.0.17`'s source — `GraphQLSubscription<E>`/
//! `GraphQLWebSocket<.., E, ..>`'s bounds are exactly `E: Executor`, nothing
//! async-graphql-axum-specific), and `flash_graphql::Schema<Q, M, S>`
//! already implements that exact trait (`crates/flash-graphql/src/
//! schema.rs`) by delegating straight to the real `dynamic::Schema`'s own
//! `execute`/`execute_stream_with_session_data`. So there is nothing for
//! this crate to wrap, patch, or reimplement — adopting it in an existing
//! async-graphql-axum codebase only ever needs the mechanical
//! `use async_graphql_axum::..` -> `use flash_graphql_axum::..` swap.
//!
//! `tests/axum_smoke.rs` is the actual proof this crate exists to provide:
//! a real `axum::Router` built from these re-exported types, serving a real
//! `flash_graphql::Schema`, driven by one real HTTP request (`tower::
//! ServiceExt::oneshot`) and one real WebSocket subscription round-trip
//! (a genuine TCP-bound server, a real `tokio-tungstenite` client speaking
//! the `graphql-transport-ws` protocol) — not just "it compiles."
pub use async_graphql_axum::*;
