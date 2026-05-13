//! PRD #10 — Default error handler emitted by the `#[endpoint]` macro.
//!
//! End-to-end coverage of three contracts:
//!
//! 1. A handler returning `Result<Response<T>, E> where E: JoltRError` and
//!    yielding `Err(e)` is converted to a JSON response with shape
//!    `{ "error": <message>, "status": <code> }` and the matching HTTP status,
//!    using the default macro-emitted Err arm (no `#[error_handler]` set).
//! 2. An `Ok(Response<T>)` from the same handler shape still bridges to the
//!    user's body as JSON with status 200.
//! 3. A `#[error_handler(fn)]` attribute on a method replaces the default Err
//!    bridge — the custom fn is called with the owned error and its return
//!    value (anything implementing `IntoResponse`) is used.
//!
//! Each endpoint registers via inventory at link time, so `JoltRServer::new()
//! .into_router()` picks it up without explicit wiring — same pattern as
//! `endpoint_integration.rs` (JOLTR-RS-045).

use axum::body::{to_bytes, Body};
use axum::http::Request as AxumRequest;
use joltr_core::{endpoint, ErrorBody, JoltRError, JoltRServer, Response, StatusCode};
use serde::Serialize;
use serde_json::Value;
use tower::ServiceExt;

// ----- Default error handler: Err(E: JoltRError) -> JSON error response -----

#[derive(Debug)]
enum DefaultApiError {
    NotFound,
}

impl JoltRError for DefaultApiError {
    fn status(&self) -> StatusCode {
        match self {
            DefaultApiError::NotFound => StatusCode::NotFound,
        }
    }
    fn message(&self) -> String {
        match self {
            DefaultApiError::NotFound => "thing not found".into(),
        }
    }
}

#[derive(Serialize)]
struct Greeting {
    hello: &'static str,
}

impl joltr_core::JsonBody for Greeting {}

#[derive(Default)]
struct DefaultErrEndpoint;

#[endpoint("/default-err")]
impl DefaultErrEndpoint {
    #[get]
    fn fetch(&self) -> Result<Response<Greeting>, DefaultApiError> {
        Err(DefaultApiError::NotFound)
    }
}

#[derive(Default)]
struct DefaultOkEndpoint;

#[endpoint("/default-ok")]
impl DefaultOkEndpoint {
    #[get]
    fn fetch(&self) -> Result<Response<Greeting>, DefaultApiError> {
        Ok(Response::new(StatusCode::Ok, Greeting { hello: "world" }))
    }
}

#[tokio::test]
async fn default_err_branch_returns_json_error_body() {
    let router = JoltRServer::new().into_router();
    let req = AxumRequest::builder()
        .method("GET")
        .uri("/default-err")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(req).await.expect("oneshot succeeds");

    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("application/json"),
    );

    let body_bytes = to_bytes(response.into_body(), 1024)
        .await
        .expect("body collects");
    let parsed: Value = serde_json::from_slice(&body_bytes).expect("body is valid JSON");
    assert_eq!(parsed["error"], "thing not found");
    assert_eq!(parsed["status"], 404);
}

#[tokio::test]
async fn default_ok_branch_still_returns_user_body() {
    // Regression: introducing the Err arm must not break the Ok path. The
    // user's body bridges to JSON exactly as the Response<T> handler did
    // before PRD #10.
    let router = JoltRServer::new().into_router();
    let req = AxumRequest::builder()
        .method("GET")
        .uri("/default-ok")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(req).await.expect("oneshot succeeds");

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 1024)
        .await
        .expect("body collects");
    let parsed: Value = serde_json::from_slice(&body_bytes).expect("body is valid JSON");
    assert_eq!(parsed["hello"], "world");
}

// ----- Custom error handler: #[error_handler(fn)] replaces default bridge -----

#[derive(Debug)]
struct CustomApiError;

// Deliberately NOT implementing JoltRError. The macro must NOT require it
// when `#[error_handler(fn)]` is supplied — that's the escape hatch's whole
// point.

/// Custom error mapper: returns a teapot status + non-default body shape, to
/// prove the default JSON `ErrorBody` bridge is bypassed. Function takes the
/// error by value (matches the macro's by-value contract — `match` arm
/// ownership of the Err binding).
fn map_custom_err(_err: CustomApiError) -> Response<ErrorBody> {
    Response::new(
        StatusCode::Other(418),
        ErrorBody {
            error: "i'm a teapot".into(),
            status: 418,
        },
    )
}

#[derive(Default)]
struct CustomErrEndpoint;

#[endpoint("/custom-err")]
impl CustomErrEndpoint {
    #[get]
    #[error_handler(map_custom_err)]
    fn fetch(&self) -> Result<Response<Greeting>, CustomApiError> {
        Err(CustomApiError)
    }
}

#[tokio::test]
async fn custom_error_handler_replaces_default_bridge() {
    let router = JoltRServer::new().into_router();
    let req = AxumRequest::builder()
        .method("GET")
        .uri("/custom-err")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(req).await.expect("oneshot succeeds");

    assert_eq!(response.status().as_u16(), 418);
    let body_bytes = to_bytes(response.into_body(), 1024)
        .await
        .expect("body collects");
    let parsed: Value = serde_json::from_slice(&body_bytes).expect("body is valid JSON");
    assert_eq!(parsed["error"], "i'm a teapot");
    assert_eq!(parsed["status"], 418);
}
