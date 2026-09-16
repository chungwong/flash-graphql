//! Hand-written end-to-end smoke test — no macros. This is the exact shape
//! `flash-graphql-derive` needs to generate mechanically for
//! `#[derive(Enum)]` (`Color`), a plain object with a nested object field
//! (`Address`, `Widget`), `#[derive(InputObject)]` (`CreateWidgetInput`), and
//! `#[Object]`-style root fields with a plain field, an arg, a nullable
//! return, and a list return (`QueryRoot`), plus a mutation taking an input
//! object (`MutationRoot`).
//!
//! It proves the trait design end-to-end through the real
//! `async_graphql::dynamic` engine: schema builds, `.sdl()` renders the
//! expected shape, and a query + a mutation both execute and produce the
//! expected JSON — not just "it compiles".

use std::borrow::Cow;

use flash_graphql::{
    Error, ID, InputType, Name, OutputType, Registrar, Result, RootFields, Schema, Value,
    dynamic::{self, FieldFuture, FieldValue},
};

// ---------------------------------------------------------------------------
// `Color` — a GraphQL enum. Implements both `OutputType` (fields typed
// `Color`) and `InputType` (arguments/input fields typed `Color`); both
// `register` calls share one `Registrar::visit::<Color>()` slot, so the enum
// is only ever registered once regardless of which side is reached first.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Color {
    Red,
    Green,
    Blue,
}

impl Color {
    const fn name(self) -> &'static str {
        match self {
            Color::Red => "RED",
            Color::Green => "GREEN",
            Color::Blue => "BLUE",
        }
    }
}

fn register_color(registrar: &mut Registrar) {
    if !registrar.visit::<Color>() {
        return;
    }
    let e = dynamic::Enum::new("Color")
        .item("RED")
        .item("GREEN")
        .item("BLUE");
    registrar.register_enum::<Color>(e);
}

impl OutputType for Color {
    fn type_name() -> Cow<'static, str> {
        Cow::Borrowed("Color")
    }

    fn register(registrar: &mut Registrar) {
        register_color(registrar);
    }

    fn resolve_owned(self) -> Result<Option<FieldValue<'static>>> {
        Ok(Some(FieldValue::value(Value::Enum(Name::new(self.name())))))
    }

    fn resolve_ref(&self) -> Result<Option<FieldValue<'_>>> {
        (*self).resolve_owned()
    }
}

impl InputType for Color {
    fn type_name() -> Cow<'static, str> {
        Cow::Borrowed("Color")
    }

    fn register(registrar: &mut Registrar) {
        register_color(registrar);
    }

    fn parse(value: Option<Value>) -> Result<Self> {
        let name = match &value {
            Some(Value::Enum(n)) => n.as_str(),
            Some(Value::String(s)) => s.as_str(),
            _ => {
                return Err(Error::new(format!(
                    "expected enum \"Color\", found {value:?}"
                )));
            }
        };
        match name {
            "RED" => Ok(Color::Red),
            "GREEN" => Ok(Color::Green),
            "BLUE" => Ok(Color::Blue),
            other => Err(Error::new(format!(
                "invalid item for enum \"Color\": \"{other}\""
            ))),
        }
    }

    fn to_value(&self) -> Value {
        Value::Enum(Name::new(self.name()))
    }
}

// ---------------------------------------------------------------------------
// `Address` — a plain nested object: one required field, one nullable field,
// no arguments anywhere, so it's a purely sync (`FieldFuture::Value`)
// `SimpleObject`-shaped type.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Address {
    city: String,
    zip: Option<String>,
}

impl OutputType for Address {
    fn type_name() -> Cow<'static, str> {
        Cow::Borrowed("Address")
    }

    fn register(registrar: &mut Registrar) {
        if !registrar.visit::<Self>() {
            return;
        }
        <String as OutputType>::register(registrar);
        <Option<String> as OutputType>::register(registrar);

        let object = dynamic::Object::new("Address")
            .field(dynamic::Field::new(
                "city",
                <String as OutputType>::type_ref(),
                |rc| {
                    FieldFuture::Value({
                        let this = rc.parent_value.try_downcast_ref::<Address>().unwrap();
                        <String as OutputType>::resolve_ref(&this.city).unwrap()
                    })
                },
            ))
            .field(dynamic::Field::new(
                "zip",
                <Option<String> as OutputType>::type_ref(),
                |rc| {
                    FieldFuture::Value({
                        let this = rc.parent_value.try_downcast_ref::<Address>().unwrap();
                        <Option<String> as OutputType>::resolve_ref(&this.zip).unwrap()
                    })
                },
            ));
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
// `Widget` — plain scalar field (`id`), enum field (`color`), list field
// (`tags`), nullable nested-object field (`address`). Also purely sync.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Widget {
    id: ID,
    name: String,
    color: Color,
    tags: Vec<String>,
    address: Option<Address>,
}

impl OutputType for Widget {
    fn type_name() -> Cow<'static, str> {
        Cow::Borrowed("Widget")
    }

    fn register(registrar: &mut Registrar) {
        if !registrar.visit::<Self>() {
            return;
        }
        <ID as OutputType>::register(registrar);
        <String as OutputType>::register(registrar);
        <Color as OutputType>::register(registrar);
        <Vec<String> as OutputType>::register(registrar);
        <Option<Address> as OutputType>::register(registrar);

        let object = dynamic::Object::new("Widget")
            .field(dynamic::Field::new(
                "id",
                <ID as OutputType>::type_ref(),
                |rc| {
                    FieldFuture::Value({
                        let this = rc.parent_value.try_downcast_ref::<Widget>().unwrap();
                        <ID as OutputType>::resolve_ref(&this.id).unwrap()
                    })
                },
            ))
            .field(dynamic::Field::new(
                "name",
                <String as OutputType>::type_ref(),
                |rc| {
                    FieldFuture::Value({
                        let this = rc.parent_value.try_downcast_ref::<Widget>().unwrap();
                        <String as OutputType>::resolve_ref(&this.name).unwrap()
                    })
                },
            ))
            .field(dynamic::Field::new(
                "color",
                <Color as OutputType>::type_ref(),
                |rc| {
                    FieldFuture::Value({
                        let this = rc.parent_value.try_downcast_ref::<Widget>().unwrap();
                        <Color as OutputType>::resolve_ref(&this.color).unwrap()
                    })
                },
            ))
            .field(dynamic::Field::new(
                "tags",
                <Vec<String> as OutputType>::type_ref(),
                |rc| {
                    FieldFuture::Value({
                        let this = rc.parent_value.try_downcast_ref::<Widget>().unwrap();
                        <Vec<String> as OutputType>::resolve_ref(&this.tags).unwrap()
                    })
                },
            ))
            .field(dynamic::Field::new(
                "address",
                <Option<Address> as OutputType>::type_ref(),
                |rc| {
                    FieldFuture::Value({
                        let this = rc.parent_value.try_downcast_ref::<Widget>().unwrap();
                        <Option<Address> as OutputType>::resolve_ref(&this.address).unwrap()
                    })
                },
            ));
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
// `CreateWidgetInput` — a GraphQL input object.
// ---------------------------------------------------------------------------

struct CreateWidgetInput {
    name: String,
    color: Color,
}

impl InputType for CreateWidgetInput {
    fn type_name() -> Cow<'static, str> {
        Cow::Borrowed("CreateWidgetInput")
    }

    fn register(registrar: &mut Registrar) {
        if !registrar.visit::<Self>() {
            return;
        }
        <String as InputType>::register(registrar);
        <Color as InputType>::register(registrar);

        let input = dynamic::InputObject::new("CreateWidgetInput")
            .field(dynamic::InputValue::new(
                "name",
                <String as InputType>::type_ref(),
            ))
            .field(dynamic::InputValue::new(
                "color",
                <Color as InputType>::type_ref(),
            ));
        registrar.register_input_object::<Self>(input);
    }

    fn parse(value: Option<Value>) -> Result<Self> {
        let Some(Value::Object(obj)) = &value else {
            return Err(Error::new(format!(
                "expected input object \"CreateWidgetInput\", found {value:?}"
            )));
        };
        Ok(CreateWidgetInput {
            name: <String as InputType>::parse(obj.get("name").cloned())?,
            color: <Color as InputType>::parse(obj.get("color").cloned())?,
        })
    }

    fn to_value(&self) -> Value {
        let mut map = async_graphql::indexmap::IndexMap::new();
        map.insert(Name::new("name"), self.name.to_value());
        map.insert(Name::new("color"), self.color.to_value());
        Value::Object(map)
    }
}

// ---------------------------------------------------------------------------
// Fake data + root objects.
// ---------------------------------------------------------------------------

fn all_widgets() -> Vec<Widget> {
    vec![
        Widget {
            id: ID("1".into()),
            name: "Widget One".into(),
            color: Color::Red,
            tags: vec!["a".into(), "b".into()],
            address: Some(Address {
                city: "Sydney".into(),
                zip: None,
            }),
        },
        Widget {
            id: ID("2".into()),
            name: "Widget Two".into(),
            color: Color::Blue,
            tags: vec![],
            address: None,
        },
    ]
}

struct QueryRoot;

impl RootFields for QueryRoot {
    const NAME: &'static str = "Query";

    fn add_fields(object: dynamic::Object) -> dynamic::Object {
        object
            .field(dynamic::Field::new(
                "ping",
                <String as OutputType>::type_ref(),
                |_rc| FieldFuture::Value(Some(FieldValue::value(Value::String("pong".into())))),
            ))
            .field(
                dynamic::Field::new("widget", <Option<Widget> as OutputType>::type_ref(), |rc| {
                    FieldFuture::Future(Box::pin(async move {
                        let id = <ID as InputType>::parse(
                            rc.args.get("id").map(|v| v.as_value().clone()),
                        )?;
                        let widget = all_widgets().into_iter().find(|w| w.id == id);
                        <Option<Widget> as OutputType>::resolve_owned(widget)
                    }))
                })
                .argument(dynamic::InputValue::new(
                    "id",
                    <ID as InputType>::type_ref(),
                )),
            )
            .field(
                dynamic::Field::new("widgets", <Vec<Widget> as OutputType>::type_ref(), |rc| {
                    FieldFuture::Future(Box::pin(async move {
                        let color = <Option<Color> as InputType>::parse(
                            rc.args.get("color").map(|v| v.as_value().clone()),
                        )?;
                        let widgets: Vec<Widget> = all_widgets()
                            .into_iter()
                            .filter(|w| color.is_none_or(|c| c == w.color))
                            .collect();
                        <Vec<Widget> as OutputType>::resolve_owned(widgets)
                    }))
                })
                .argument(dynamic::InputValue::new(
                    "color",
                    <Option<Color> as InputType>::type_ref(),
                )),
            )
    }

    fn register(registrar: &mut Registrar) {
        <String as OutputType>::register(registrar);
        <Option<Widget> as OutputType>::register(registrar);
        <ID as InputType>::register(registrar);
        <Vec<Widget> as OutputType>::register(registrar);
        <Option<Color> as InputType>::register(registrar);
    }
}

struct MutationRoot;

impl RootFields for MutationRoot {
    const NAME: &'static str = "Mutation";

    fn add_fields(object: dynamic::Object) -> dynamic::Object {
        object.field(
            dynamic::Field::new("createWidget", <Widget as OutputType>::type_ref(), |rc| {
                FieldFuture::Future(Box::pin(async move {
                    let input = <CreateWidgetInput as InputType>::parse(
                        rc.args.get("input").map(|v| v.as_value().clone()),
                    )?;
                    let widget = Widget {
                        id: ID("3".into()),
                        name: input.name,
                        color: input.color,
                        tags: vec![],
                        address: None,
                    };
                    <Widget as OutputType>::resolve_owned(widget)
                }))
            })
            .argument(dynamic::InputValue::new(
                "input",
                <CreateWidgetInput as InputType>::type_ref(),
            )),
        )
    }

    fn register(registrar: &mut Registrar) {
        <Widget as OutputType>::register(registrar);
        <CreateWidgetInput as InputType>::register(registrar);
    }
}

// ---------------------------------------------------------------------------
// The test itself.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn builds_and_executes() {
    let schema = Schema::build(QueryRoot, MutationRoot, flash_graphql::EmptySubscription)
        .finish()
        .expect("schema should build");

    let sdl = schema.sdl();
    assert!(sdl.contains("type Widget"), "sdl:\n{sdl}");
    assert!(sdl.contains("type Address"), "sdl:\n{sdl}");
    assert!(sdl.contains("enum Color"), "sdl:\n{sdl}");
    assert!(sdl.contains("input CreateWidgetInput"), "sdl:\n{sdl}");
    assert!(sdl.contains("ping: String!"), "sdl:\n{sdl}");
    println!("--- SDL ---\n{sdl}");

    let query = r#"{
        ping
        widget(id: "1") { id name color tags address { city zip } }
        widgets(color: BLUE) { id name }
    }"#;
    let response = schema.execute(query).await;
    assert!(response.errors.is_empty(), "errors: {:#?}", response.errors);
    let json: serde_json::Value = serde_json::to_value(&response.data).unwrap();
    println!(
        "--- query result ---\n{}",
        serde_json::to_string_pretty(&json).unwrap()
    );
    assert_eq!(json["ping"], "pong");
    assert_eq!(json["widget"]["name"], "Widget One");
    assert_eq!(json["widget"]["color"], "RED");
    assert_eq!(json["widget"]["tags"], serde_json::json!(["a", "b"]));
    assert_eq!(json["widget"]["address"]["city"], "Sydney");
    assert!(json["widget"]["address"]["zip"].is_null());
    // `widgets(color: BLUE)` must filter out "Widget One" (RED) and keep only
    // "Widget Two" (BLUE) — proves `Option<Color>` argument parsing and the
    // list resolver both round-tripped correctly, not just that *a* list
    // came back.
    let widgets = json["widgets"].as_array().unwrap();
    assert_eq!(widgets.len(), 1, "widgets: {widgets:#?}");
    assert_eq!(widgets[0]["name"], "Widget Two");

    let mutation = r#"mutation {
        createWidget(input: { name: "New Widget", color: GREEN }) { id name color }
    }"#;
    let response = schema.execute(mutation).await;
    assert!(response.errors.is_empty(), "errors: {:#?}", response.errors);
    let json: serde_json::Value = serde_json::to_value(&response.data).unwrap();
    println!(
        "--- mutation result ---\n{}",
        serde_json::to_string_pretty(&json).unwrap()
    );
    assert_eq!(json["createWidget"]["name"], "New Widget");
    assert_eq!(json["createWidget"]["color"], "GREEN");
}

#[test]
fn duplicate_type_name_is_a_clear_error_not_a_silent_overwrite() {
    struct A;
    struct B;

    impl OutputType for A {
        fn type_name() -> Cow<'static, str> {
            Cow::Borrowed("Clashing")
        }
        fn register(registrar: &mut Registrar) {
            if !registrar.visit::<Self>() {
                return;
            }
            registrar.register_object::<Self>(dynamic::Object::new("Clashing").field(
                dynamic::Field::new("a", <String as OutputType>::type_ref(), |_| {
                    FieldFuture::Value(Some(FieldValue::value(Value::String("a".into()))))
                }),
            ));
        }
        fn resolve_owned(self) -> Result<Option<FieldValue<'static>>> {
            Ok(Some(FieldValue::owned_any(self)))
        }
    }

    impl OutputType for B {
        fn type_name() -> Cow<'static, str> {
            Cow::Borrowed("Clashing")
        }
        fn register(registrar: &mut Registrar) {
            if !registrar.visit::<Self>() {
                return;
            }
            registrar.register_object::<Self>(dynamic::Object::new("Clashing").field(
                dynamic::Field::new("b", <String as OutputType>::type_ref(), |_| {
                    FieldFuture::Value(Some(FieldValue::value(Value::String("b".into()))))
                }),
            ));
        }
        fn resolve_owned(self) -> Result<Option<FieldValue<'static>>> {
            Ok(Some(FieldValue::owned_any(self)))
        }
    }

    struct ClashingQuery;
    impl RootFields for ClashingQuery {
        const NAME: &'static str = "Query";
        fn add_fields(object: dynamic::Object) -> dynamic::Object {
            object
                .field(dynamic::Field::new(
                    "a",
                    <A as OutputType>::type_ref(),
                    |_| FieldFuture::Value(Some(FieldValue::owned_any(A))),
                ))
                .field(dynamic::Field::new(
                    "b",
                    <B as OutputType>::type_ref(),
                    |_| FieldFuture::Value(Some(FieldValue::owned_any(B))),
                ))
        }
        fn register(registrar: &mut Registrar) {
            <A as OutputType>::register(registrar);
            <B as OutputType>::register(registrar);
        }
    }

    let result = Schema::build(
        ClashingQuery,
        flash_graphql::EmptyMutation,
        flash_graphql::EmptySubscription,
    )
    .finish();
    let err = match result {
        Ok(_) => panic!("two Rust types claiming the same GraphQL name must not build silently"),
        Err(e) => e,
    };

    let message = err.0;
    assert!(message.contains("Clashing"), "message: {message}");
    assert!(message.contains("claimed by both"), "message: {message}");
}
