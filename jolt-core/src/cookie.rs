//! Cookie value type used by `Request::cookies`.
//!
//! Minimal stub: just the name/value pair. Parsing, attributes (Path, Domain,
//! Secure, etc.), and Set-Cookie serialization will be added by later PRD
//! items as concrete consumers arrive. Kept dependency-free so we don't pin
//! a cookie crate version before there's a real need.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
}
