//! Outbound HTTP response value returned from Jolt handlers.

use axum::body::Body;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, HeaderValue};
use serde::Serialize;

use crate::status::StatusCode;

#[derive(Debug, Clone)]
pub struct Response<T> {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: T,
}

impl<T> Response<T> {
    pub fn new(status: StatusCode, body: T) -> Self {
        Response {
            status,
            headers: HeaderMap::new(),
            body,
        }
    }
}

/// Marker trait identifying body types whose `Response<T>` bridges to axum
/// as `application/json`. Notably NOT implemented for `String`/`&str`, which
/// route through their own `text/plain` impls.
///
/// Downstream crates opt in their own structs by writing
/// `impl JsonBody for MyStruct {}` (the `Serialize` bound carries through).
pub trait JsonBody: Serialize {}

impl JsonBody for () {}
impl JsonBody for bool {}
impl JsonBody for i8 {}
impl JsonBody for i16 {}
impl JsonBody for i32 {}
impl JsonBody for i64 {}
impl JsonBody for u8 {}
impl JsonBody for u16 {}
impl JsonBody for u32 {}
impl JsonBody for u64 {}
impl JsonBody for f32 {}
impl JsonBody for f64 {}
impl JsonBody for serde_json::Value {}

fn finalize_axum_response(
    status: StatusCode,
    extra_headers: HeaderMap,
    body: Body,
    content_type: HeaderValue,
) -> axum::response::Response {
    let mut response = axum::response::Response::builder()
        .status(axum::http::StatusCode::from(status))
        .body(body)
        .expect("axum::response::Response builder accepts a status and body");

    let response_headers = response.headers_mut();
    response_headers.extend(extra_headers);
    response_headers.insert(CONTENT_TYPE, content_type);

    response
}

impl<T: JsonBody> From<Response<T>> for axum::response::Response {
    fn from(value: Response<T>) -> Self {
        let Response {
            status,
            headers,
            body,
        } = value;

        let json = serde_json::to_vec(&body)
            .expect("Response<T> body must serialize to JSON when bridging to axum");

        finalize_axum_response(
            status,
            headers,
            Body::from(json),
            HeaderValue::from_static("application/json"),
        )
    }
}

impl From<Response<String>> for axum::response::Response {
    fn from(value: Response<String>) -> Self {
        let Response {
            status,
            headers,
            body,
        } = value;

        finalize_axum_response(
            status,
            headers,
            Body::from(body),
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )
    }
}

impl<'a> From<Response<&'a str>> for axum::response::Response {
    fn from(value: Response<&'a str>) -> Self {
        let Response {
            status,
            headers,
            body,
        } = value;

        finalize_axum_response(
            status,
            headers,
            Body::from(body.to_string()),
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )
    }
}
