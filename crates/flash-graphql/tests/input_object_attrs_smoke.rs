//! End-to-end test of the deferred-then-added `InputObject`/`SimpleObject`
//! field attrs: `input_name` (container-level), `validator(email)`,
//! `process_with = fn_name` (both bare-path and string-literal spellings,
//! seen used side by side in real-world code), and `secret` (accepted,
//! currently inert — see `util.rs`'s `FieldAttrs` doc comment). Shapes match
//! real-world usage directly (a filter-input module's `#[graphql(input_name
//! = "..")]` structs, a user-input module's `UserInput`/`LoginInput`
//! fields, a `Model::password_hash`-style field's `#[graphql(secret,
//! skip)]`).

use flash_graphql::{ComplexObject, EmptySubscription, InputObject, Object, Schema, SimpleObject};

/// A typical real-world shape: a Rust type name that differs from its real
/// GraphQL input type name.
#[derive(Debug, InputObject)]
#[graphql(input_name = "RegionFilter")]
struct UnsafeRegionFilter {
    tenant: String,
}

fn trim_username(username: &mut String) {
    *username = username.trim().to_string();
}

fn str_trim_lowercase(s: &mut String) {
    *s = s.trim().to_lowercase();
}

/// A typical real-world shape: `validator(email)` +
/// `process_with` (string-literal spelling) on the same field.
#[derive(Debug, InputObject)]
struct UserInput {
    #[graphql(validator(email), process_with = "str_trim_lowercase")]
    email: String,
    first_name: String,
}

/// A typical real-world shape: `process_with` (bare-path
/// spelling), no validator.
#[derive(Debug, InputObject)]
struct LoginInput {
    #[graphql(process_with = trim_username)]
    username: String,
}

/// A typical real-world shape: `secret`
/// paired with `skip` on a `SimpleObject` field.
#[derive(SimpleObject)]
#[graphql(complex)]
struct Account {
    email: String,
    #[graphql(secret, skip)]
    #[allow(dead_code)]
    password_hash: String,
}

#[ComplexObject]
impl Account {
    async fn label(&self) -> String {
        format!("account:{}", self.email)
    }
}

#[derive(Default)]
struct QueryRoot;

#[Object]
impl QueryRoot {
    async fn filter(&self, filter: UnsafeRegionFilter) -> String {
        filter.tenant
    }

    async fn account(&self) -> Account {
        Account {
            email: "a@example.com".into(),
            password_hash: "hunter2".into(),
        }
    }
}

#[derive(Default)]
struct MutationRoot;

#[Object]
impl MutationRoot {
    async fn create_user(&self, input: UserInput) -> String {
        format!("{}/{}", input.email, input.first_name)
    }

    async fn login(&self, input: LoginInput) -> String {
        input.username
    }
}

fn build_schema() -> Schema<QueryRoot, MutationRoot, EmptySubscription> {
    Schema::build(QueryRoot, MutationRoot, EmptySubscription)
        .finish()
        .expect("schema should build")
}

#[tokio::test]
async fn input_name_renames_the_sdl_type_and_still_parses() {
    let schema = build_schema();
    let sdl = schema.sdl();
    assert!(sdl.contains("input RegionFilter"), "sdl:\n{sdl}");
    // Not `!sdl.contains("UnsafeRegionFilter")` — container-level doc-comment
    // descriptions are supported, and this type's own doc comment mentions
    // the Rust name in prose (see `UnsafeRegionFilter`'s doc comment above),
    // so that substring legitimately appears in the SDL now. What actually
    // matters is that the *type declaration* itself uses the renamed name.
    assert!(!sdl.contains("input UnsafeRegionFilter"), "sdl:\n{sdl}");

    let response = schema
        .execute(r#"{ filter(filter: { tenant: "AU" }) }"#)
        .await;
    assert!(response.errors.is_empty(), "errors: {:#?}", response.errors);
    let json: serde_json::Value = serde_json::to_value(&response.data).unwrap();
    assert_eq!(json["filter"], "AU");
}

#[tokio::test]
async fn validator_email_accepts_valid_and_rejects_invalid() {
    let schema = build_schema();

    let ok = schema
        .execute(
            r#"mutation { createUser(input: { email: "  Joe@Example.com ", firstName: "Joe" }) }"#,
        )
        .await;
    assert!(ok.errors.is_empty(), "errors: {:#?}", ok.errors);
    let json: serde_json::Value = serde_json::to_value(&ok.data).unwrap();
    // `process_with = "str_trim_lowercase"` ran after the validator, on the
    // same field — trimmed and lowercased.
    assert_eq!(json["createUser"], "joe@example.com/Joe");

    let bad = schema
        .execute(r#"mutation { createUser(input: { email: "not-an-email", firstName: "Joe" }) }"#)
        .await;
    assert!(
        !bad.errors.is_empty(),
        "expected a validation error for a malformed email"
    );
    assert!(
        bad.errors[0].message.contains("invalid"),
        "{:#?}",
        bad.errors
    );
}

#[tokio::test]
async fn process_with_bare_path_runs_before_use() {
    let schema = build_schema();
    let response = schema
        .execute(r#"mutation { login(input: { username: "  bob  " }) }"#)
        .await;
    assert!(response.errors.is_empty(), "errors: {:#?}", response.errors);
    let json: serde_json::Value = serde_json::to_value(&response.data).unwrap();
    assert_eq!(json["login"], "bob");
}

#[tokio::test]
async fn secret_field_is_accepted_and_stays_excluded_via_skip() {
    let schema = build_schema();
    let sdl = schema.sdl();
    assert!(!sdl.contains("passwordHash"), "sdl:\n{sdl}");

    let response = schema.execute("{ account { email label } }").await;
    assert!(response.errors.is_empty(), "errors: {:#?}", response.errors);
    let json: serde_json::Value = serde_json::to_value(&response.data).unwrap();
    assert_eq!(json["account"]["email"], "a@example.com");
    assert_eq!(json["account"]["label"], "account:a@example.com");
}
