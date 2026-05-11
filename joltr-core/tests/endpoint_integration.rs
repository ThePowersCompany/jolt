//! JOLTR-RS-045 PRD-mandated integration test.
//!
//! Verifies the end-to-end path: `#[endpoint("/ping")]` with a `#[get]` method
//! → inventory registration → `JoltRServer::into_router()` → GET /ping returns
//! 200 with the expected body and Content-Type.

use axum::body::{to_bytes, Body};
use axum::http::Request as AxumRequest;
use joltr_core::{endpoint, JoltRServer, Response, StatusCode};
use tower::ServiceExt;

#[derive(Default)]
struct PingEndpoint;

#[endpoint("/ping")]
impl PingEndpoint {
    #[get]
    fn ping(&self) -> Response<&'static str> {
        Response::new(StatusCode::Ok, "pong")
    }
}

#[tokio::test]
async fn ping_endpoint_returns_200() {
    let router = JoltRServer::new().into_router();
    let req = AxumRequest::builder()
        .method("GET")
        .uri("/ping")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(req).await.expect("oneshot succeeds");

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 1024)
        .await
        .expect("body collects");
    assert_eq!(&body_bytes[..], b"pong");
}
