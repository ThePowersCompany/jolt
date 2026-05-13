//! Shared utilities for the JoltR framework: password hashing (pbkdf2 + sha2),
//! JWT signing/verification (jsonwebtoken), UUID/time helpers (uuid + chrono),
//! and serde glue.
//!
//! The [`jwt`] module supports typed claims and HS/RS/ES signing algorithms.
//! The remaining modules provide password hashing, UUID generation, datetime
//! formatting, email/mime validation helpers, and JSON wrapper types shared by
//! the core framework crates.

pub mod datetime;
pub mod email;
pub mod json_types;
pub mod jwt;
pub mod mime;
pub mod password;
pub mod uuid;

pub use json_types::{Json, JsonArray};

#[cfg(test)]
mod tests;
