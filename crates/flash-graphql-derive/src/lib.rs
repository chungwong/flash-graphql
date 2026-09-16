//! `flash-graphql-derive` — proc macros with async-graphql's exact names and
//! attribute syntax (`#[derive(SimpleObject)]`, `#[Object]`,
//! `#[ComplexObject]`, `#[derive(Enum)]`, `#[derive(InputObject)]`,
//! `#[derive(MergedObject)]`), mechanically generating the same shape
//! `flash-graphql`'s `tests/smoke.rs` hand-writes: `impl OutputType`/
//! `InputType` for user types in terms of `flash_graphql::{Registrar,
//! OutputType, InputType, RootFields, Flatten, ComplexObjectFields}` and the
//! re-exported `async_graphql::dynamic` engine.
//!
//! Always used through `flash-graphql`'s facade (`pub use
//! flash_graphql_derive::{..}` in `flash-graphql/src/lib.rs`) — generated
//! code hardcodes the absolute path `::flash_graphql::..` rather than doing
//! `proc-macro-crate` name detection, since that's the only place this crate
//! is ever consumed from (see `util::crate_path`).
//!
extern crate proc_macro;

mod complex_object;
mod enum_type;
mod input_object;
mod interface;
mod merged_object;
mod object;
mod oneof_object;
mod simple_object;
mod subscription;
mod util;

use proc_macro::TokenStream;

#[proc_macro_derive(Enum, attributes(graphql))]
pub fn derive_enum(input: TokenStream) -> TokenStream {
    enum_type::derive(input)
}

#[proc_macro_derive(SimpleObject, attributes(graphql))]
pub fn derive_simple_object(input: TokenStream) -> TokenStream {
    simple_object::derive(input)
}

#[proc_macro_attribute]
#[allow(non_snake_case)]
pub fn Object(args: TokenStream, input: TokenStream) -> TokenStream {
    object::attribute(args, input)
}

#[proc_macro_attribute]
#[allow(non_snake_case)]
pub fn ComplexObject(args: TokenStream, input: TokenStream) -> TokenStream {
    complex_object::attribute(args, input)
}

#[proc_macro_derive(InputObject, attributes(graphql))]
pub fn derive_input_object(input: TokenStream) -> TokenStream {
    input_object::derive(input)
}

#[proc_macro_derive(MergedObject, attributes(graphql))]
pub fn derive_merged_object(input: TokenStream) -> TokenStream {
    merged_object::derive(input)
}

#[proc_macro_derive(Interface, attributes(graphql))]
pub fn derive_interface(input: TokenStream) -> TokenStream {
    interface::derive(input)
}

#[proc_macro_derive(OneofObject, attributes(graphql))]
pub fn derive_oneof_object(input: TokenStream) -> TokenStream {
    oneof_object::derive(input)
}

#[proc_macro_attribute]
#[allow(non_snake_case)]
pub fn Subscription(args: TokenStream, input: TokenStream) -> TokenStream {
    subscription::attribute(args, input)
}
