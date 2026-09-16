use std::{borrow::Cow, collections::HashSet, hash::Hash};

use async_graphql::{Error, Result, Value, dynamic::TypeRef};

use crate::Registrar;

/// The compile-time typed counterpart of a GraphQL input type (an argument or
/// an input object field).
///
/// Deviation worth flagging: an earlier sketch of this design wrote this as
/// `parse(Option<Value>) -> InputValueResult<Self>`. The real
/// `InputValueResult<T> = Result<T, InputValueError<T>>` (`error.rs` in
/// `async-graphql-7.0.17`) can only be *constructed* — `expected_type`,
/// `custom`, `propagate`, ... — when `T: async_graphql::InputType`, the
/// crate's own top-level trait with the heavier `create_type_info`/registry
/// machinery this crate exists to avoid putting on user types. Every dynamic
/// engine boundary that actually produces argument values already speaks
/// plain `async_graphql::Result<T>` (`ValueAccessor::deserialize`,
/// `ResolverContext`'s `FieldFuture::Future` body, ...), so `parse` returns
/// that here instead — it's the type the rest of this design already needs
/// to produce anyway (`?` inside a resolver body), not a new one.
pub trait InputType: Sized + Send + Sync + 'static {
    /// The bare GraphQL type name (`"Int"`, `"MyInput"`, ...).
    fn type_name() -> Cow<'static, str>;

    /// The full type reference as it appears in argument/input-field position
    /// (`Int!`, `[MyInput!]`, ...). See `OutputType::type_ref` — same
    /// default, same `Option<T>` override.
    fn type_ref() -> TypeRef {
        TypeRef::NonNull(Box::new(TypeRef::named(Self::type_name())))
    }

    /// Register this type — and everything it transitively depends on
    /// (input object field types) — into `registrar`. Same `visit`-first
    /// contract as `OutputType::register`; in fact for a type that is both
    /// (an enum, most commonly) the two share the same `Registrar::visited`
    /// entry, so it's only ever built once regardless of which trait's
    /// `register` runs first.
    fn register(registrar: &mut Registrar);

    /// Parse from an already-validated argument/input-field `Value`. `None`
    /// means the argument was entirely absent. By the time this runs,
    /// async-graphql's (concrete, compiled-once) request validator has
    /// already rejected a missing/null value for anything the schema marked
    /// non-null with no default — so a plain (non-`Option`) `T::parse`
    /// seeing `None`/`Value::Null` is a defensive error path, not the common
    /// case; `Option<T>` is what actually handles optional arguments.
    fn parse(value: Option<Value>) -> Result<Self>;

    /// Convert back to a `Value` (introspection default-value printing,
    /// `OneofObject`/default-value plumbing, tests, ...).
    fn to_value(&self) -> Value;
}

impl<T: InputType> InputType for Option<T> {
    fn type_name() -> Cow<'static, str> {
        T::type_name()
    }

    fn type_ref() -> TypeRef {
        match T::type_ref() {
            TypeRef::NonNull(inner) => *inner,
            other => other,
        }
    }

    fn register(registrar: &mut Registrar) {
        T::register(registrar);
    }

    fn parse(value: Option<Value>) -> Result<Self> {
        match value {
            None | Some(Value::Null) => Ok(None),
            some => T::parse(some).map(Some),
        }
    }

    fn to_value(&self) -> Value {
        match self {
            Some(v) => v.to_value(),
            None => Value::Null,
        }
    }
}

impl<T: InputType> InputType for Vec<T> {
    fn type_name() -> Cow<'static, str> {
        T::type_name()
    }

    fn type_ref() -> TypeRef {
        TypeRef::NonNull(Box::new(TypeRef::List(Box::new(T::type_ref()))))
    }

    fn register(registrar: &mut Registrar) {
        T::register(registrar);
    }

    fn parse(value: Option<Value>) -> Result<Self> {
        let value = match value {
            None | Some(Value::Null) => {
                return Err(Error::new(format!(
                    "expected a non-null value of type \"{}\"",
                    Self::type_name()
                )));
            }
            Some(v) => v,
        };
        match value {
            Value::List(items) => items.into_iter().map(|item| T::parse(Some(item))).collect(),
            other => Err(Error::new(format!(
                "expected a list for type \"{}\", found {other}",
                Self::type_name()
            ))),
        }
    }

    fn to_value(&self) -> Value {
        Value::List(self.iter().map(InputType::to_value).collect())
    }
}

/// A recursive input object (its own `not`/`and`/`or` fields nest
/// arbitrarily deep, e.g. a field typed `Option<Box<Self>>`) needs `Box` to
/// have a known size at all. Delegates to `T` exactly like
/// `OutputType for Box<T>` does.
impl<T: InputType> InputType for Box<T> {
    fn type_name() -> Cow<'static, str> {
        T::type_name()
    }

    fn type_ref() -> TypeRef {
        T::type_ref()
    }

    fn register(registrar: &mut Registrar) {
        T::register(registrar);
    }

    fn parse(value: Option<Value>) -> Result<Self> {
        T::parse(value).map(Box::new)
    }

    fn to_value(&self) -> Value {
        (**self).to_value()
    }
}

/// A fixed-size array input, e.g. a date-range/between-style filter field
/// typed `[String; 2]`, `[f32; 2]`, or `[DateTime<Local>; 2]`. Same SDL
/// shape as `Vec<T>` (real async-graphql's own `[T; N]` impl also reports
/// `type_name() = "[T]"`, i.e. a plain GraphQL list — arity is a Rust-side
/// contract only, checked here at parse time, not part of the schema), so
/// `type_ref`/`register` delegate to `Vec<T>` and `parse` just adds the
/// length check `Vec<T>` doesn't need.
impl<T: InputType, const N: usize> InputType for [T; N] {
    fn type_name() -> Cow<'static, str> {
        T::type_name()
    }

    fn type_ref() -> TypeRef {
        <Vec<T> as InputType>::type_ref()
    }

    fn register(registrar: &mut Registrar) {
        T::register(registrar);
    }

    fn parse(value: Option<Value>) -> Result<Self> {
        let items = <Vec<T> as InputType>::parse(value)?;
        let len = items.len();
        items.try_into().map_err(|_| {
            Error::new(format!(
                "expected a list of exactly {N} items for type \"[{}]\", found {len}",
                T::type_name(),
            ))
        })
    }

    fn to_value(&self) -> Value {
        Value::List(self.iter().map(InputType::to_value).collect())
    }
}

/// A `HashSet<T>` argument (e.g. a bulk-lookup-by-code argument typed
/// `HashSet<String>`) — same GraphQL-list shape as `Vec<T>` above (a plain
/// `[String!]!` in the schema, not a distinct GraphQL type); a duplicate
/// argument value just collapses into the set, same as it would duplicate
/// a `Vec` entry the resolver body has to dedupe itself.
impl<T: InputType + Eq + Hash> InputType for HashSet<T> {
    fn type_name() -> Cow<'static, str> {
        T::type_name()
    }

    fn type_ref() -> TypeRef {
        TypeRef::NonNull(Box::new(TypeRef::List(Box::new(T::type_ref()))))
    }

    fn register(registrar: &mut Registrar) {
        T::register(registrar);
    }

    fn parse(value: Option<Value>) -> Result<Self> {
        let value = match value {
            None | Some(Value::Null) => {
                return Err(Error::new(format!(
                    "expected a non-null value of type \"{}\"",
                    Self::type_name()
                )));
            }
            Some(v) => v,
        };
        match value {
            Value::List(items) => items.into_iter().map(|item| T::parse(Some(item))).collect(),
            other => Err(Error::new(format!(
                "expected a list for type \"{}\", found {other}",
                Self::type_name()
            ))),
        }
    }

    fn to_value(&self) -> Value {
        Value::List(self.iter().map(InputType::to_value).collect())
    }
}
