# flash-graphql

A code-first GraphQL server library for Rust with async-graphql's macro
surface (`#[Object]`, `#[derive(SimpleObject)]`, `InputObject`, `Enum`,
`Interface`, `OneofObject`, `MergedObject`, `#[Subscription]`, guards, Relay
connections, dataloader) — but backed by async-graphql's type-erased
`dynamic` engine instead of its monomorphized, code-first derives.

The problem this solves: async-graphql's static derives generate resolver
code that is heavily monomorphized *inside your crate* — every `#[Object]`/
`SimpleObject` type re-instantiates a chunk of async-graphql's generic
engine (field dispatch, list/optional resolution, input parsing), and
`MergedObject` root types nest generically per member. In a large real
schema this makes every incremental `cargo build` re-pay that cost, however
small the actual edit. flash-graphql's macros generate small, non-generic
glue code (a boxed closure per field) that calls into async-graphql's
`dynamic` engine, which is compiled once as a dependency and never touches
your crate — while your authoring surface stays fully typed and checked by
rustc at compile time, same as async-graphql's static side.

## Status

Feature-complete for the schema shape a large real production GraphQL API
(several hundred types) actually uses: `#[Object]`, `SimpleObject`/
`#[ComplexObject]`, `Enum`, `InputObject` (`name`, `skip`, `desc`, `flatten`,
`complex`, `default`/`default = expr`, `input_name`, `validator(email)`,
`process_with`, `secret`), flat `MergedObject`, `Interface` (with
implements-patching), `OneofObject`, `#[Subscription]`, guards
(`.and()`/`.or()` composition), a real generic Relay `Connection`/`Edge`/
`PageInfo` module, dataloader (re-exported, proven to batch), and
`flash-graphql-axum` (a genuine zero-work re-export of `async-graphql-axum`,
proven against real HTTP requests and a real WebSocket subscription
round-trip).

Ported and benchmarked against a real ~19K-line production GraphQL crate
(a separate private codebase, not included here): the full crate builds
clean, SDL output is byte-identical to the original async-graphql schema,
and incremental rebuilds after a one-line resolver edit are ~1.6–2.3x
faster once migration-period scaffolding (dual-derives kept temporarily so
half-ported code still builds) is fully removed.

async-graphql's `dynamic` engine, as shipped, doesn't isolate a resolver
error to just its nullable field (an error anywhere propagates to
`data: null` instead of nulling just that field) and resolves sibling
object fields serially rather than concurrently. Both are fixed via a
small, documented patch — see `vendor/PATCH.md`.

## Layout

- `crates/flash-graphql` — core traits (`OutputType`, `InputType`), the
  `Registrar`, the `Schema<Q, M, S>` facade over `dynamic::Schema`, scalars,
  the `Connection`/`Edge`/`PageInfo` module.
- `crates/flash-graphql-derive` — the proc macros.
- `crates/flash-graphql-axum` — HTTP/WebSocket glue (a thin re-export).
- `vendor/async-graphql-7.0.17` — a trimmed local copy of async-graphql
  7.0.17 (just `src/` plus its license/changelog), patched per
  `vendor/PATCH.md` and wired in via `[patch.crates-io]`.

## Testing

`cargo test --workspace` runs the full suite (schema construction, SDL
shape, guards, connections, interfaces, oneof inputs, subscriptions,
dataloader batching, and a real HTTP + WebSocket round-trip via
`flash-graphql-axum`).
