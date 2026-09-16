//! `OutputType`/`InputType` for GraphQL's built-in scalars (`Int`, `Float`,
//! `String`, `Boolean`, `ID`) plus the handful of custom scalars a real-world
//! schema needs (`DateTime`, `NaiveDate`, `JSON`). None of these ever call
//! `Registrar::register_scalar` for the five built-ins: `dynamic::Schema`
//! synthesizes `Int`/`Float`/`Boolean`/`String`/`ID` itself in `finish()`
//! (`dynamic/schema.rs`), so registering them ourselves would be redundant
//! (and, since `finish()` unconditionally re-inserts them, harmless either
//! way — but a no-op is clearer).

use std::borrow::Cow;

use async_graphql::{
    Error, ID, Result, Value,
    dynamic::{FieldValue, TypeRef},
};

use crate::{InputType, OutputType, Registrar};

macro_rules! impl_int_scalar {
    ($ty:ty) => {
        impl OutputType for $ty {
            fn type_name() -> Cow<'static, str> {
                Cow::Borrowed(TypeRef::INT)
            }
            fn register(_registrar: &mut Registrar) {}
            fn resolve_owned(self) -> Result<Option<FieldValue<'static>>> {
                Ok(Some(FieldValue::value(self)))
            }
            fn resolve_ref(&self) -> Result<Option<FieldValue<'_>>> {
                Ok(Some(FieldValue::value(*self)))
            }
        }

        impl InputType for $ty {
            fn type_name() -> Cow<'static, str> {
                Cow::Borrowed(TypeRef::INT)
            }
            fn register(_registrar: &mut Registrar) {}
            fn parse(value: Option<Value>) -> Result<Self> {
                match &value {
                    Some(Value::Number(n)) => n.as_i64().and_then(|n| <$ty>::try_from(n).ok()),
                    _ => None,
                }
                .ok_or_else(|| Error::new(format!("expected Int, found {value:?}")))
            }
            fn to_value(&self) -> Value {
                Value::from(*self)
            }
        }
    };
}

impl_int_scalar!(i8);
impl_int_scalar!(i16);
impl_int_scalar!(i32);
impl_int_scalar!(i64);
impl_int_scalar!(u8);
impl_int_scalar!(u16);
impl_int_scalar!(u32);
// `usize`/`u64` come up for real in practice — e.g. a connection-paging arg
// typed `Option<usize>` in Rust but declared `count: Int` in the schema
// (same "Int" scalar as the other integer widths above, not a distinct
// GraphQL type).
impl_int_scalar!(usize);
impl_int_scalar!(u64);

macro_rules! impl_float_scalar {
    ($ty:ty) => {
        impl OutputType for $ty {
            fn type_name() -> Cow<'static, str> {
                Cow::Borrowed(TypeRef::FLOAT)
            }
            fn register(_registrar: &mut Registrar) {}
            fn resolve_owned(self) -> Result<Option<FieldValue<'static>>> {
                Ok(Some(FieldValue::value(self)))
            }
            fn resolve_ref(&self) -> Result<Option<FieldValue<'_>>> {
                Ok(Some(FieldValue::value(*self)))
            }
        }

        impl InputType for $ty {
            fn type_name() -> Cow<'static, str> {
                Cow::Borrowed(TypeRef::FLOAT)
            }
            fn register(_registrar: &mut Registrar) {}
            fn parse(value: Option<Value>) -> Result<Self> {
                match &value {
                    Some(Value::Number(n)) => n.as_f64().map(|n| n as $ty),
                    _ => None,
                }
                .ok_or_else(|| Error::new(format!("expected Float, found {value:?}")))
            }
            fn to_value(&self) -> Value {
                Value::from(*self)
            }
        }
    };
}

impl_float_scalar!(f32);
impl_float_scalar!(f64);

impl OutputType for bool {
    fn type_name() -> Cow<'static, str> {
        Cow::Borrowed(TypeRef::BOOLEAN)
    }
    fn register(_registrar: &mut Registrar) {}
    fn resolve_owned(self) -> Result<Option<FieldValue<'static>>> {
        Ok(Some(FieldValue::value(self)))
    }
    fn resolve_ref(&self) -> Result<Option<FieldValue<'_>>> {
        Ok(Some(FieldValue::value(*self)))
    }
}

impl InputType for bool {
    fn type_name() -> Cow<'static, str> {
        Cow::Borrowed(TypeRef::BOOLEAN)
    }
    fn register(_registrar: &mut Registrar) {}
    fn parse(value: Option<Value>) -> Result<Self> {
        match value {
            Some(Value::Boolean(b)) => Ok(b),
            other => Err(Error::new(format!("expected Boolean, found {other:?}"))),
        }
    }
    fn to_value(&self) -> Value {
        Value::Boolean(*self)
    }
}

impl OutputType for String {
    fn type_name() -> Cow<'static, str> {
        Cow::Borrowed(TypeRef::STRING)
    }
    fn register(_registrar: &mut Registrar) {}
    fn resolve_owned(self) -> Result<Option<FieldValue<'static>>> {
        Ok(Some(FieldValue::value(Value::String(self))))
    }
    fn resolve_ref(&self) -> Result<Option<FieldValue<'_>>> {
        Ok(Some(FieldValue::value(Value::String(self.clone()))))
    }
}

impl InputType for String {
    fn type_name() -> Cow<'static, str> {
        Cow::Borrowed(TypeRef::STRING)
    }
    fn register(_registrar: &mut Registrar) {}
    fn parse(value: Option<Value>) -> Result<Self> {
        match value {
            Some(Value::String(s)) => Ok(s),
            other => Err(Error::new(format!("expected String, found {other:?}"))),
        }
    }
    fn to_value(&self) -> Value {
        Value::String(self.clone())
    }
}

impl OutputType for ID {
    fn type_name() -> Cow<'static, str> {
        Cow::Borrowed(TypeRef::ID)
    }
    fn register(_registrar: &mut Registrar) {}
    fn resolve_owned(self) -> Result<Option<FieldValue<'static>>> {
        Ok(Some(FieldValue::value(Value::String(self.0))))
    }
    fn resolve_ref(&self) -> Result<Option<FieldValue<'_>>> {
        Ok(Some(FieldValue::value(Value::String(self.0.clone()))))
    }
}

impl InputType for ID {
    fn type_name() -> Cow<'static, str> {
        Cow::Borrowed(TypeRef::ID)
    }
    fn register(_registrar: &mut Registrar) {}
    fn parse(value: Option<Value>) -> Result<Self> {
        match value {
            Some(Value::String(s)) => Ok(ID(s)),
            Some(Value::Number(n)) if n.is_i64() => Ok(ID(n.to_string())),
            other => Err(Error::new(format!("expected ID, found {other:?}"))),
        }
    }
    fn to_value(&self) -> Value {
        Value::String(self.0.clone())
    }
}

#[cfg(feature = "chrono")]
mod chrono_scalars {
    use super::*;

    /// Matches real async-graphql-derive's `#[Scalar]` doc comment, which
    /// differs per `Tz` impl (`"Implement the DateTime<Utc> scalar"` /
    /// `"...Local..."` / `"...FixedOffset..."`, each followed by `"The
    /// input/output is a string in RFC3339 format."` —
    /// `types/external/datetime.rs`). Whichever `Tz` a concrete schema's
    /// registry happens to register first is the one whose description ends
    /// up in the exported SDL (`DateTime<Local>` is used here as the
    /// representative case) — reproduced verbatim since a bare dynamic
    /// `Scalar::new("DateTime")` carries no description of its own, and this
    /// crate's `register_shared_scalar` only ever registers once regardless
    /// of which `Tz` gets there first (see that fn's doc comment).
    const DATE_TIME_DESCRIPTION: &str =
        "Implement the DateTime<Local> scalar\n\nThe input/output is a string in RFC3339 format.";

    /// async-graphql's own doc comment on `impl ScalarType for NaiveDate`
    /// (`types/external/naive_time.rs`), reproduced verbatim.
    const NAIVE_DATE_DESCRIPTION: &str = "ISO 8601 calendar date without timezone.\nFormat: %Y-%m-%d\n\n# Examples\n\n* `1994-11-13`\n* `2000-02-24`";

    /// Shared by every `DateTime<Tz>` instantiation below — see
    /// `Registrar::register_shared_scalar`'s doc comment for why a marker
    /// key (not `chrono::DateTime<Utc>` itself) is needed: `DateTime<Local>`
    /// (used, for example, by date-range/between-style filter fields) shares
    /// the same `DateTime` GraphQL name as `DateTime<Utc>`, which
    /// `register_scalar::<T>`'s per-`T` claim would have rejected as a
    /// collision the moment a second `Tz` registered.
    struct DateTimeScalarKey;

    /// Real async-graphql's own `#[Scalar(specified_by_url = "..")]` on
    /// every `DateTime<Tz>` impl. `specified_by_url` is never *inline*-
    /// annotated on `scalar DateTime` itself (that needs the exporter's
    /// `include_specified_by` option, which a plain `.sdl()` call doesn't
    /// set) — but a scalar merely *declaring* one is what real async-
    /// graphql's SDL exporter checks to decide whether to print the
    /// built-in `directive @specifiedBy(url: String!) on SCALAR` definition
    /// at all (`registry/export_sdl.rs`), which is otherwise filtered out
    /// as unused. Verified byte-for-byte against a real schema's exported
    /// SDL.
    const DATE_TIME_SPECIFIED_BY_URL: &str = "https://datatracker.ietf.org/doc/html/rfc3339";

    fn register_date_time(registrar: &mut Registrar) {
        registrar.register_shared_scalar::<DateTimeScalarKey>(
            async_graphql::dynamic::Scalar::new("DateTime")
                .description(DATE_TIME_DESCRIPTION)
                .specified_by_url(DATE_TIME_SPECIFIED_BY_URL),
        );
    }

    /// One `impl` per `Tz`, all printing as the same `DateTime` scalar —
    /// matches real async-graphql's own separate `ScalarType` impls for
    /// `DateTime<Utc>`/`DateTime<Local>`/`DateTime<FixedOffset>`
    /// (`types/external/datetime.rs`).
    macro_rules! impl_datetime {
        ($tz:ty) => {
            impl OutputType for chrono::DateTime<$tz> {
                fn type_name() -> Cow<'static, str> {
                    Cow::Borrowed("DateTime")
                }
                fn register(registrar: &mut Registrar) {
                    register_date_time(registrar);
                }
                fn resolve_owned(self) -> Result<Option<FieldValue<'static>>> {
                    Ok(Some(FieldValue::value(Value::String(self.to_rfc3339()))))
                }
                fn resolve_ref(&self) -> Result<Option<FieldValue<'_>>> {
                    Ok(Some(FieldValue::value(Value::String(self.to_rfc3339()))))
                }
            }

            impl InputType for chrono::DateTime<$tz> {
                fn type_name() -> Cow<'static, str> {
                    Cow::Borrowed("DateTime")
                }
                fn register(registrar: &mut Registrar) {
                    register_date_time(registrar);
                }
                fn parse(value: Option<Value>) -> Result<Self> {
                    match value {
                        Some(Value::String(s)) => s
                            .parse::<chrono::DateTime<$tz>>()
                            .map_err(|e| Error::new(e.to_string())),
                        other => Err(Error::new(format!("expected DateTime, found {other:?}"))),
                    }
                }
                fn to_value(&self) -> Value {
                    Value::String(self.to_rfc3339())
                }
            }
        };
    }

    impl_datetime!(chrono::Utc);
    impl_datetime!(chrono::Local);
    impl_datetime!(chrono::FixedOffset);

    fn register_naive_date(registrar: &mut Registrar) {
        if !registrar.visit::<chrono::NaiveDate>() {
            return;
        }
        registrar.register_scalar::<chrono::NaiveDate>(
            async_graphql::dynamic::Scalar::new("NaiveDate").description(NAIVE_DATE_DESCRIPTION),
        );
    }

    impl OutputType for chrono::NaiveDate {
        fn type_name() -> Cow<'static, str> {
            Cow::Borrowed("NaiveDate")
        }
        fn register(registrar: &mut Registrar) {
            register_naive_date(registrar);
        }
        fn resolve_owned(self) -> Result<Option<FieldValue<'static>>> {
            Ok(Some(FieldValue::value(Value::String(
                self.format("%Y-%m-%d").to_string(),
            ))))
        }
        fn resolve_ref(&self) -> Result<Option<FieldValue<'_>>> {
            Ok(Some(FieldValue::value(Value::String(
                self.format("%Y-%m-%d").to_string(),
            ))))
        }
    }

    impl InputType for chrono::NaiveDate {
        fn type_name() -> Cow<'static, str> {
            Cow::Borrowed("NaiveDate")
        }
        fn register(registrar: &mut Registrar) {
            register_naive_date(registrar);
        }
        fn parse(value: Option<Value>) -> Result<Self> {
            match value {
                Some(Value::String(s)) => chrono::NaiveDate::parse_from_str(&s, "%Y-%m-%d")
                    .map_err(|e| Error::new(e.to_string())),
                other => Err(Error::new(format!("expected NaiveDate, found {other:?}"))),
            }
        }
        fn to_value(&self) -> Value {
            Value::String(self.format("%Y-%m-%d").to_string())
        }
    }
}

/// A scalar that can represent any JSON value — `Json<T>` for any
/// `T: Serialize + DeserializeOwned`, matching async-graphql's own
/// `async_graphql::Json<T>` (re-exported here) and its exact description
/// (`types/json.rs`: `"A scalar that can represent any JSON value."`).
pub use async_graphql::Json;

const JSON_DESCRIPTION: &str = "A scalar that can represent any JSON value.";

/// Shared by every `Json<T>` instantiation (any `T`) *and* the bare
/// `serde_json::Value` impl below — see
/// `Registrar::register_shared_scalar`'s doc comment: `Json<T1>` and
/// `Json<T2>` for two different `T`s, or `Json<T>` alongside a bare
/// `serde_json::Value` field elsewhere in the same schema, all print as the
/// same `JSON` GraphQL scalar and must not collide.
struct JsonScalarKey;

fn register_json(registrar: &mut Registrar) {
    registrar.register_shared_scalar::<JsonScalarKey>(
        async_graphql::dynamic::Scalar::new("JSON").description(JSON_DESCRIPTION),
    );
}

impl<T> OutputType for Json<T>
where
    T: serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static,
{
    fn type_name() -> Cow<'static, str> {
        Cow::Borrowed("JSON")
    }

    fn register(registrar: &mut Registrar) {
        register_json(registrar);
    }

    fn resolve_owned(self) -> Result<Option<FieldValue<'static>>> {
        let value = async_graphql::to_value(&self.0).map_err(|e| Error::new(e.to_string()))?;
        Ok(Some(FieldValue::value(value)))
    }

    fn resolve_ref(&self) -> Result<Option<FieldValue<'_>>> {
        let value = async_graphql::to_value(&self.0).map_err(|e| Error::new(e.to_string()))?;
        Ok(Some(FieldValue::value(value)))
    }
}

impl<T> InputType for Json<T>
where
    T: serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static,
{
    fn type_name() -> Cow<'static, str> {
        Cow::Borrowed("JSON")
    }

    fn register(registrar: &mut Registrar) {
        register_json(registrar);
    }

    fn parse(value: Option<Value>) -> Result<Self> {
        let value = value.unwrap_or(Value::Null);
        async_graphql::from_value(value)
            .map(Json)
            .map_err(|e| Error::new(e.to_string()))
    }

    fn to_value(&self) -> Value {
        async_graphql::to_value(&self.0).unwrap_or(Value::Null)
    }
}

/// A *bare* `serde_json::Value` field (as commonly produced by an ORM's
/// JSON column type, e.g. `sea_orm::entity::prelude::Json`, itself
/// `pub use serde_json::Value as Json` — not the `Json<T>` wrapper above)
/// needs its own impl, matching real async-graphql's own bespoke
/// `impl OutputType for serde_json::Value` (`async-graphql-7.0.17`'s
/// `src/types/json.rs:107,144` — genuinely separate from its `Json<T>` impl,
/// not derived from it). Registers the same `"JSON"` scalar name as
/// `Json<T>`, via the shared `JsonScalarKey` (fixed proactively — see
/// `Registrar::register_shared_scalar`, added for the exact same class of
/// bug hit for real between `DateTime<Utc>` and `DateTime<Local>`, two
/// widely-used timezone variants that both print as the same `DateTime`
/// scalar).
fn register_json_value(registrar: &mut Registrar) {
    registrar.register_shared_scalar::<JsonScalarKey>(
        async_graphql::dynamic::Scalar::new("JSON").description(JSON_DESCRIPTION),
    );
}

impl OutputType for serde_json::Value {
    fn type_name() -> Cow<'static, str> {
        Cow::Borrowed("JSON")
    }

    fn register(registrar: &mut Registrar) {
        register_json_value(registrar);
    }

    fn resolve_owned(self) -> Result<Option<FieldValue<'static>>> {
        let value = async_graphql::to_value(&self).map_err(|e| Error::new(e.to_string()))?;
        Ok(Some(FieldValue::value(value)))
    }

    fn resolve_ref(&self) -> Result<Option<FieldValue<'_>>> {
        let value = async_graphql::to_value(self).map_err(|e| Error::new(e.to_string()))?;
        Ok(Some(FieldValue::value(value)))
    }
}

impl InputType for serde_json::Value {
    fn type_name() -> Cow<'static, str> {
        Cow::Borrowed("JSON")
    }

    fn register(registrar: &mut Registrar) {
        register_json_value(registrar);
    }

    fn parse(value: Option<Value>) -> Result<Self> {
        let value = value.unwrap_or(Value::Null);
        async_graphql::from_value(value).map_err(|e| Error::new(e.to_string()))
    }

    fn to_value(&self) -> Value {
        async_graphql::to_value(self).unwrap_or(Value::Null)
    }
}
