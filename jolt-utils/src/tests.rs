//! Comprehensive unit tests for jolt-utils features.
//!
//! Each feature (JWT, password, UUID, etc.) has its own submodule so the
//! per-feature test filter from the PRD works as a stable target:
//! `cargo test -p jolt-utils -- tests::jwt`.
//!
//! New tests cover scenarios the inline `#[cfg(test)] mod tests` blocks in the
//! source modules don't yet exercise. When a submodule duplicates an assertion
//! already present inline, it does so intentionally — the per-submodule
//! filterable shape ensures the PRD verification command succeeds independently
//! of whatever live in inline test modules.

mod jwt;
