//! PRD item 1 — redirect response helper.

use axum::body::to_bytes;
use axum::http::header::LOCATION;
use joltr_core::response::Redirect;
use joltr_core::{Response, StatusCode};

#[test]
fn response_redirect_sets_status_and_location_header() {
    let response = Response::redirect("/dashboard", StatusCode::Other(302));

    assert_eq!(response.status, StatusCode::Other(302));
    assert_eq!(
        response.headers.get(LOCATION).and_then(|v| v.to_str().ok()),
        Some("/dashboard")
    );
    assert!(response.body.is_empty());
}

#[test]
fn redirect_type_converts_into_response() {
    let redirect = Redirect::new("https://example.com/new", StatusCode::Other(303));

    assert_eq!(redirect.location(), "https://example.com/new");
    assert_eq!(redirect.status(), StatusCode::Other(303));

    let response: Response<String> = redirect.into();
    assert_eq!(response.status, StatusCode::Other(303));
    assert_eq!(
        response.headers.get(LOCATION).and_then(|v| v.to_str().ok()),
        Some("https://example.com/new")
    );
}

#[test]
fn supports_standard_redirect_status_codes() {
    for status in [301, 302, 303, 307, 308] {
        let response = Response::redirect("/target", StatusCode::Other(status));
        assert_eq!(response.status.as_u16(), status);
    }
}

#[test]
#[should_panic(expected = "redirect status must be one of 301, 302, 303, 307, or 308")]
fn rejects_non_redirect_status_codes() {
    let _ = Response::redirect("/not-a-redirect", StatusCode::Ok);
}

#[tokio::test]
async fn bridges_to_axum_response_with_location_header() {
    let response: axum::response::Response =
        Response::redirect("/temporary", StatusCode::Other(307)).into();

    assert_eq!(
        response.status(),
        axum::http::StatusCode::TEMPORARY_REDIRECT
    );
    assert_eq!(
        response
            .headers()
            .get(LOCATION)
            .and_then(|v| v.to_str().ok()),
        Some("/temporary")
    );

    let body_bytes = to_bytes(response.into_body(), 1024)
        .await
        .expect("body collects");
    assert!(body_bytes.is_empty());
}
