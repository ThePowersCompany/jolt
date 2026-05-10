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

impl<T: Serialize> From<Response<T>> for axum::response::Response {
    fn from(value: Response<T>) -> Self {
        let Response {
            status,
            headers,
            body,
        } = value;

        let json = serde_json::to_vec(&body)
            .expect("Response<T> body must serialize to JSON when bridging to axum");

        let mut response = axum::response::Response::builder()
            .status(axum::http::StatusCode::from(status))
            .body(Body::from(json))
            .expect("axum::response::Response builder accepts a status and body");

        let response_headers = response.headers_mut();
        response_headers.extend(headers);
        response_headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        response
    }
}
