//! Inbound HTTP request value passed to Jolt handlers.
//!
//! Field-only definition for JOLT-RS-011. Accessors (`header`, `query`,
//! `json`, `cookie`, `has_finished`) land in JOLT-RS-012..014.

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
