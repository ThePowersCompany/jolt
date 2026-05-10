//! Inbound HTTP request value passed to Jolt handlers.

use std::collections::HashMap;

use axum::http::HeaderMap;

use crate::cookie::Cookie;
use crate::method::Method;

#[derive(Debug, Clone)]
pub struct Request {
    pub method: Method,
    pub path: String,
    pub headers: HeaderMap,
    pub query_params: HashMap<String, String>,
    pub body: Vec<u8>,
    pub cookies: Vec<Cookie>,
}

impl Request {
    /// Look up a header by name. Case-insensitive per RFC 9110 §5.1; values
    /// containing non-visible-ASCII bytes return `None`.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
}
