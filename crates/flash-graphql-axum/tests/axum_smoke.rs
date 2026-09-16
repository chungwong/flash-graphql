//! Proves `flash-graphql-axum`'s whole reason to exist: real
//! `async-graphql-axum` extractors/handlers, wired unmodified against a real
//! `flash_graphql::Schema`, actually serve real requests — not just "it
//! compiles."
//!
//! - `real_http_query_executes_against_the_router`: a real `axum::Router`
//!   using `GraphQLRequest`/`GraphQLResponse`, driven by one real HTTP POST
//!   via `tower::ServiceExt::oneshot` (no network — an in-process
//!   `tower::Service` call, but a genuine `http::Request`/`http::Response`
//!   round trip through axum's own extraction/response machinery).
//! - `real_websocket_subscription_round_trip`: the same router's `/graphql/ws`
//!   route (`GraphQLProtocol`/`GraphQLWebSocket`), bound to a real
//!   `TcpListener` and served via `axum::serve`, driven by a real
//!   `tokio-tungstenite` client speaking the `graphql-transport-ws` protocol
//!   end to end: `connection_init` -> `connection_ack` -> `subscribe` -> a
//!   `next` message per stream item -> `complete`.
use axum::{
    Router,
    body::Body,
    extract::{State, WebSocketUpgrade},
    http::{Request, StatusCode},
    response::Response,
    routing::{get, post},
};
use flash_graphql::{EmptyMutation, Object, Result, Schema, Subscription};
use flash_graphql_axum::{GraphQLProtocol, GraphQLRequest, GraphQLResponse, GraphQLWebSocket};
use futures_util::Stream;
use tower::ServiceExt;

#[derive(Default)]
struct QueryRoot;

#[Object]
impl QueryRoot {
    async fn ping(&self) -> String {
        "pong".into()
    }
}

#[derive(Default)]
struct SubscriptionRoot;

#[Subscription]
impl SubscriptionRoot {
    async fn counter(&self) -> Result<impl Stream<Item = i32>> {
        Ok(futures_util::stream::iter(vec![1, 2, 3]))
    }
}

type TestSchema = Schema<QueryRoot, EmptyMutation, SubscriptionRoot>;

fn build_schema() -> TestSchema {
    Schema::build(QueryRoot, EmptyMutation, SubscriptionRoot)
        .finish()
        .expect("schema should build")
}

/// Matches a typical real-world `graphql_handler` shape almost exactly,
/// minus the auth/session extractors that are orthogonal to what this crate
/// proves.
async fn graphql_handler(State(schema): State<TestSchema>, req: GraphQLRequest) -> GraphQLResponse {
    schema.execute(req.into_inner()).await.into()
}

/// Matches a typical real-world `graphql_ws_handler` shape, same simplification.
async fn ws_handler(
    State(schema): State<TestSchema>,
    protocol: GraphQLProtocol,
    ws: WebSocketUpgrade,
) -> Response {
    ws.protocols(["graphql-transport-ws", "graphql-ws"])
        .on_upgrade(move |stream| GraphQLWebSocket::new(stream, schema, protocol).serve())
}

fn build_router(schema: TestSchema) -> Router {
    Router::new()
        .route("/graphql", post(graphql_handler))
        .route("/graphql/ws", get(ws_handler))
        .with_state(schema)
}

#[tokio::test]
async fn real_http_query_executes_against_the_router() {
    let router = build_router(build_schema());

    let body = serde_json::json!({ "query": "{ ping }" }).to_string();
    let request = Request::builder()
        .method("POST")
        .uri("/graphql")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["data"]["ping"], "pong", "response body: {json:#?}");
}

#[tokio::test]
async fn real_websocket_subscription_round_trip() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

    let router = build_router(build_schema());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let mut request = format!("ws://{addr}/graphql/ws")
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        "graphql-transport-ws".parse().unwrap(),
    );
    let (mut ws, handshake_response) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(
        handshake_response
            .headers()
            .get("sec-websocket-protocol")
            .and_then(|v| v.to_str().ok()),
        Some("graphql-transport-ws"),
        "server should have echoed back the negotiated subprotocol"
    );

    ws.send(Message::text(r#"{"type":"connection_init"}"#))
        .await
        .unwrap();
    let ack = ws.next().await.unwrap().unwrap();
    assert!(
        ack.to_text().unwrap().contains("connection_ack"),
        "expected connection_ack, got {ack:?}"
    );

    ws.send(Message::text(
        r#"{"type":"subscribe","id":"1","payload":{"query":"subscription { counter }"}}"#,
    ))
    .await
    .unwrap();

    let mut values = Vec::new();
    loop {
        let msg = ws.next().await.unwrap().unwrap();
        let text = msg.to_text().unwrap();
        let json: serde_json::Value = serde_json::from_str(text).unwrap();
        match json["type"].as_str().unwrap() {
            "next" => values.push(json["payload"]["data"]["counter"].as_i64().unwrap()),
            "complete" => break,
            other => panic!("unexpected message type {other}: {json}"),
        }
    }
    assert_eq!(values, vec![1, 2, 3]);

    ws.close(None).await.ok();
    server.abort();
}
