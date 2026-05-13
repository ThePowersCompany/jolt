//! PRD item 9 — `JoltRError` trait surface and JSON-error conversion.
//!
//! Exercises the contract documented on the trait: a typed error implementing
//! `JoltRError` produces a `Response<ErrorBody>` whose status matches
//! `status()` and whose JSON body has shape `{ "error": <message>, "status":
//! <code> }`. The conversion to `axum::response::Response` rides the existing
//! `From<Response<T>> for axum::response::Response` bridge — this test
//! verifies that bridge serves error bodies without any extra wiring.

use axum::body::to_bytes;
use joltr_core::{ErrorBody, JoltRError, Response, StatusCode};
use serde_json::Value;

#[derive(Debug)]
enum ApiError {
    NotFound { resource: String },
    BadRequest(&'static str),
    Internal,
}

impl JoltRError for ApiError {
    fn status(&self) -> StatusCode {
        match self {
            ApiError::NotFound { .. } => StatusCode::NotFound,
            ApiError::BadRequest(_) => StatusCode::BadRequest,
            ApiError::Internal => StatusCode::InternalServerError,
        }
    }

    fn message(&self) -> String {
        match self {
            ApiError::NotFound { resource } => format!("{} not found", resource),
            ApiError::BadRequest(reason) => (*reason).to_string(),
            ApiError::Internal => "internal server error".to_string(),
        }
    }
}

#[test]
fn to_response_carries_status_and_body_fields() {
    let err = ApiError::NotFound {
        resource: "user".into(),
    };
    let response: Response<ErrorBody> = err.to_response();

    assert_eq!(response.status, StatusCode::NotFound);
    assert_eq!(response.body.error, "user not found");
    assert_eq!(response.body.status, 404);
}

#[tokio::test]
async fn bridges_to_axum_response_as_json() {
    let err = ApiError::BadRequest("missing field `name`");
    let response: axum::response::Response = err.to_response().into();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
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
    assert_eq!(parsed["error"], "missing field `name`");
    assert_eq!(parsed["status"], 400);
}

#[test]
fn internal_error_maps_to_500() {
    let response = ApiError::Internal.to_response();
    assert_eq!(response.status, StatusCode::InternalServerError);
    assert_eq!(response.body.status, 500);
    assert_eq!(response.body.error, "internal server error");
}

/// Confirms a custom `to_response` override (e.g. attaching response headers)
/// composes with the trait's default body shape.
#[test]
fn custom_to_response_can_attach_headers() {
    struct Unauthorized;

    impl JoltRError for Unauthorized {
        fn status(&self) -> StatusCode {
            StatusCode::Unauthorized
        }
        fn message(&self) -> String {
            "auth required".into()
        }
        fn to_response(&self) -> Response<ErrorBody> {
            let mut response = Response::new(
                self.status(),
                ErrorBody {
                    error: self.message(),
                    status: self.status().as_u16(),
                },
            );
            response.headers.insert(
                axum::http::header::WWW_AUTHENTICATE,
                axum::http::HeaderValue::from_static("Bearer"),
            );
            response
        }
    }

    let response = Unauthorized.to_response();
    assert_eq!(response.status, StatusCode::Unauthorized);
    assert_eq!(
        response
            .headers
            .get(axum::http::header::WWW_AUTHENTICATE)
            .and_then(|v| v.to_str().ok()),
        Some("Bearer"),
    );
}
