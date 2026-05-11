//! JOLT-RS-110 / JOLT-RS-111 / JOLT-RS-112 / JOLT-RS-114 PRD-mandated
//! integration tests.
//!
//! Verifies that `#[derive(PatchQuery)]` compiles on a struct with
//! `Optional<T>` fields (110), the struct-level `#[patch("table")]`
//! attribute is parsed and emitted (111), `Optional<T>` fields are
//! detected and counted (112), and the generated `to_patch_query` method
//! compiles and type-checks (114).
//!
//! This is an integration test (not a unit test) because the derive macro can
//! only be exercised through cargo's compile pipeline. The proc-macro crate's
//! own unit tests parse-check the emitted token stream but cannot expand and
//! type-check the derive against a real `DeriveInput` from a downstream crate.
//!
//! The hidden consts emitted by the derive are the observable witnesses:
//! - `__JOLT_PATCH_QUERY_FIELD_COUNT: usize` (110) — total named field count.
//! - `__JOLT_PATCH_QUERY_OPTIONAL_COUNT: usize` (112) — count of fields whose
//!   type matches `Optional<T>`.
//! - `__JOLT_PATCH_QUERY_TABLE_NAME` (111) — parsed `#[patch(...)]` value.
//!
//! Uses `jolt_core::Optional` (the real framework tri-state enum, introduced
//! in JOLT-RS-114) and `jolt_core::ToSql` so the generated `to_patch_query`
//! method type-checks against the real framework types.

use jolt_core::{Optional, PatchQuery};

/// Unit-style patch target: zero fields. The derive must accept this and
/// report a field count of 0.
#[derive(PatchQuery)]
struct EmptyPatch;

/// The PRD-mandated surface: a struct with `Optional<T>` fields using
/// `jolt_core::Optional`. All inner types implement `ToSql`.
#[derive(PatchQuery)]
#[allow(dead_code)]
struct UserPatch {
    name: Optional<String>,
    email: Optional<String>,
    age: Optional<u32>,
    bio: Optional<String>,
}

/// Mixed Optional and non-Optional fields. Plain fields must implement
/// `ToSql` so the generated `to_patch_query` method compiles.
#[derive(PatchQuery)]
#[allow(dead_code)]
struct MixedPatch {
    title: Optional<String>,
    view_count: u64,
    tags: String,
    body: Optional<String>,
}

// ── JOLT-RS-111: #[patch("table_name")] attribute ──

#[derive(PatchQuery)]
#[patch("users")]
#[allow(dead_code)]
struct UserTablePatch {
    name: Optional<String>,
    email: Optional<String>,
}

// ── JOLT-RS-114: to_patch_query method compiles ──

#[derive(PatchQuery)]
#[patch("accounts")]
#[allow(dead_code)]
struct AccountPatch {
    display_name: Optional<String>,
    bio: String,
    follower_count: u64,
}

#[test]
fn empty_patch_derive_emits_zero_field_count() {
    assert_eq!(EmptyPatch::__JOLT_PATCH_QUERY_FIELD_COUNT, 0);
}

#[test]
fn user_patch_derive_counts_all_optional_fields() {
    assert_eq!(UserPatch::__JOLT_PATCH_QUERY_FIELD_COUNT, 4);
    assert_eq!(UserPatch::__JOLT_PATCH_QUERY_OPTIONAL_COUNT, 4);
}

#[test]
fn mixed_patch_derive_counts_optional_and_plain_fields_together() {
    assert_eq!(MixedPatch::__JOLT_PATCH_QUERY_FIELD_COUNT, 4);
    assert_eq!(MixedPatch::__JOLT_PATCH_QUERY_OPTIONAL_COUNT, 2);
}

#[test]
fn table_patch_derive_emits_table_name() {
    assert_eq!(UserTablePatch::__JOLT_PATCH_QUERY_TABLE_NAME, "users");
}

#[test]
fn empty_patch_missing_table_returns_none() {
    assert!(EmptyPatch::__JOLT_PATCH_QUERY_TABLE_NAME.is_none());
}

// ── JOLT-RS-114: to_patch_query compiles and returns correct shape ──

#[test]
fn to_patch_query_generates_sql_string_and_params() {
    let patch = AccountPatch {
        display_name: Optional::NotProvided,
        bio: "Hello world".to_string(),
        follower_count: 42,
    };
    let (sql, params) = patch.to_patch_query("id", &1u64);
    assert!(sql.starts_with("UPDATE"), "SQL must be an UPDATE, got: {sql}");
    assert!(sql.contains("accounts"), "SQL must reference table name, got: {sql}");
    assert!(!params.is_empty(), "params must include at least the id_value");
}

#[test]
fn to_patch_query_optional_some_includes_column_in_set_clause() {
    let patch = AccountPatch {
        display_name: Optional::Some("Alice".to_string()),
        bio: "Engineer".to_string(),
        follower_count: 0,
    };
    let (sql, params) = patch.to_patch_query("id", &99u64);
    assert!(sql.contains("display_name"), "Some field must appear in SET clause, got: {sql}");
    // All four params: display_name, bio, follower_count, id_value
    assert_eq!(params.len(), 4, "expected 4 params (3 SET + 1 WHERE), got: {sql}");
}

#[test]
fn to_patch_query_optional_null_sets_column_to_null() {
    let patch = AccountPatch {
        display_name: Optional::Null,
        bio: "".to_string(),
        follower_count: 0,
    };
    let (sql, _params) = patch.to_patch_query("id", &1u64);
    assert!(sql.contains("display_name = NULL"), "Null field must produce SET col = NULL, got: {sql}");
}

#[test]
fn to_patch_query_optional_not_provided_skips_column() {
    let patch = AccountPatch {
        display_name: Optional::NotProvided,
        bio: "".to_string(),
        follower_count: 0,
    };
    let (sql, params) = patch.to_patch_query("id", &1u64);
    // Only bio, follower_count, id_value — display_name is skipped
    assert!(!sql.contains("display_name"), "NotProvided field must be absent from SET, got: {sql}");
    assert_eq!(params.len(), 3, "expected 3 params (2 SET + 1 WHERE), got: {sql}");
}

#[test]
fn to_patch_query_where_clause_uses_id_column() {
    let patch = AccountPatch {
        display_name: Optional::NotProvided,
        bio: "x".to_string(),
        follower_count: 1,
    };
    let (sql, _params) = patch.to_patch_query("user_id", &42u64);
    assert!(sql.contains("WHERE user_id"), "WHERE clause must use id_column, got: {sql}");
}
