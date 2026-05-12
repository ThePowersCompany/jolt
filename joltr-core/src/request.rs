//! Inbound HTTP request value passed to JoltR handlers.

use std::collections::HashMap;

use axum::http::HeaderMap;
use serde::de::DeserializeOwned;

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
    pub finished: bool,
}

impl Request {
    /// Look up a header by name. Case-insensitive per RFC 9110 §5.1; values
    /// containing non-visible-ASCII bytes return `None`.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }

    /// Look up a query parameter by exact key match.
    pub fn query(&self, key: &str) -> Option<&str> {
        self.query_params.get(key).map(String::as_str)
    }

    /// Deserialize the raw request body as JSON into `T`.
    pub fn json<T: DeserializeOwned>(&self) -> serde_json::Result<T> {
        serde_json::from_slice(&self.body)
    }

    /// Look up a cookie by exact name match. Returns the first matching cookie.
    pub fn cookie(&self, name: &str) -> Option<&Cookie> {
        self.cookies.iter().find(|c| c.name == name)
    }

    /// Parse all cookies from the Cookie request header.
    pub fn cookies(&self) -> Vec<Cookie> {
        self.header("Cookie")
            .map(Cookie::parse_all)
            .unwrap_or_default()
    }

    /// Whether middleware has marked this request as finished, signaling
    /// downstream layers to skip further handler dispatch.
    pub fn has_finished(&self) -> bool {
        self.finished
    }
}
