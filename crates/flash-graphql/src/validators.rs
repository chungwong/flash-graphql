//! `#[graphql(validator(email))]` support (deferred from an earlier pass,
//! now added) — validating that a user-supplied input field is a
//! syntactically valid email address is a common real-world need (e.g. a
//! signup or profile-update input's `email` field).
//!
//! Real async-graphql's own `async_graphql::validators::email` (`src/
//! validators/email.rs`, `fast_chemail`-backed) is bounded by `T: AsRef<str>
//! + async_graphql::InputType` — the heavy top-level trait this whole
//! design exists to avoid requiring on user types (see this crate's own
//! `InputType` doc comment). Since the actual check is just "is this string
//! shaped like an email address", reimplementing it against the same
//! `fast_chemail` crate real async-graphql depends on needs no such bound.
use crate::{Error, Result};

/// Validate that `value` is a syntactically valid email address, matching
/// real async-graphql's own `fast_chemail::is_valid_email`-backed check
/// (same error message, `"invalid email"`) byte for byte.
pub fn email(value: &str) -> Result<()> {
    if fast_chemail::is_valid_email(value) {
        Ok(())
    } else {
        Err(Error::new("invalid email"))
    }
}
