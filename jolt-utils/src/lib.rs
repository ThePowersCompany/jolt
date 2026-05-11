//! Shared utilities for the Jolt framework: password hashing (pbkdf2 + sha2),
//! JWT signing/verification (jsonwebtoken), UUID/time helpers (uuid + chrono),
//! and serde glue.
//!
//! The [`jwt`] module landed with JOLT-RS-072 (decode + typed claims + typed
//! error variants); password hashing, UUID helpers, and serde glue land in
//! subsequent PRD items.

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
