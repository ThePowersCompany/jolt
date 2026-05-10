//! Outbound HTTP response value returned from Jolt handlers.

use axum::http::HeaderMap;

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
