//! JOLTR-RS-110 / JOLTR-RS-111 / JOLTR-RS-112 / JOLTR-RS-114 / JOLTR-RS-117
//! integration tests.
//!
//! Verifies that `#[derive(PatchQuery)]` compiles on a struct with
//! `Optional<T>` fields (110), the struct-level `#[patch("table")]`
//! attribute is parsed and emitted (111), `Optional<T>` fields are
//! detected and counted (112), and the generated `to_patch_query` method
//! compiles and type-checks (114).
//!
//! JOLTR-RS-117 (closing test bundle): all-fields-Some, all-Null, mixed
//! Some+Null+NotProvided, empty struct no-op, and table_name attribute gap
//! coverage.
//!
//! This is an integration test (not a unit test) because the derive macro can
//! only be exercised through cargo's compile pipeline. The proc-macro crate's
//! own unit tests parse-check the emitted token stream but cannot expand and
//! type-check the derive against a real `DeriveInput` from a downstream crate.
//!
//! The hidden consts emitted by the derive are the observable witnesses:
//! - `__JOLTR_PATCH_QUERY_FIELD_COUNT: usize` (110) — total named field count.
//! - `__JOLTR_PATCH_QUERY_OPTIONAL_COUNT: usize` (112) — count of fields whose
//!   type matches `Optional<T>`.
//! - `__JOLTR_PATCH_QUERY_TABLE_NAME` (111) — parsed `#[patch(...)]` value.
//!
//! Uses `joltr_core::Optional` (the real framework tri-state enum, introduced
//! in JOLTR-RS-114) and `joltr_core::ToSql` so the generated `to_patch_query`
//! method type-checks against the real framework types.

use joltr_core::{Optional, PatchQuery};

/// Unit-style patch target: zero fields. The derive must accept this and
/// report a field count of 0.
#[derive(PatchQuery)]
struct EmptyPatch;

/// The PRD-mandated surface: a struct with `Optional<T>` fields using
/// `joltr_core::Optional`. All inner types implement `ToSql`.
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

// ── JOLTR-RS-111: #[patch("table_name")] attribute ──

#[derive(PatchQuery)]
#[patch("users")]
#[allow(dead_code)]
struct UserTablePatch {
    name: Optional<String>,
    email: Optional<String>,
}

// ── JOLTR-RS-114: to_patch_query method compiles ──

#[derive(PatchQuery)]
#[patch("accounts")]
#[allow(dead_code)]
struct AccountPatch {
    display_name: Optional<String>,
    bio: String,
    follower_count: u64,
}

// ── JOLTR-RS-117: closing test bundle structs ──

/// All four fields are Optional. Used for all-Some and all-Null tests.
#[derive(PatchQuery)]
#[patch("media")]
#[allow(dead_code)]
struct AllOptionalPatch {
    title: Optional<String>,
    description: Optional<String>,
    tags: Optional<String>,
    url: Optional<String>,
}

/// Four Optional fields. Used for the mixed Some+Null+NotProvided test
/// where each field takes a different tri-state variant in one call.
#[derive(PatchQuery)]
#[patch("items")]
#[allow(dead_code)]
struct MixedStatePatch {
    name: Optional<String>,
    color: Optional<String>,
    size: Optional<String>,
    weight: Optional<String>,
}

/// Unit struct WITH a table name. The empty-struct no-op test exercises
/// the early-return path when no fields can contribute to the SET clause.
#[derive(PatchQuery)]
#[patch("logs")]
#[allow(dead_code)]
struct EmptyWithTablePatch;

/// A struct with a table name, used to lock in the gap that the runtime
/// SQL round-trips the parsed attribute correctly when fields are present.
#[derive(PatchQuery)]
#[patch("customers")]
#[allow(dead_code)]
struct CustomersPatch {
    email: Optional<String>,
    name: Optional<String>,
}

#[test]
fn empty_patch_derive_emits_zero_field_count() {
    assert_eq!(EmptyPatch::__JOLTR_PATCH_QUERY_FIELD_COUNT, 0);
}

#[test]
fn user_patch_derive_counts_all_optional_fields() {
    assert_eq!(UserPatch::__JOLTR_PATCH_QUERY_FIELD_COUNT, 4);
    assert_eq!(UserPatch::__JOLTR_PATCH_QUERY_OPTIONAL_COUNT, 4);
}

#[test]
fn mixed_patch_derive_counts_optional_and_plain_fields_together() {
    assert_eq!(MixedPatch::__JOLTR_PATCH_QUERY_FIELD_COUNT, 4);
    assert_eq!(MixedPatch::__JOLTR_PATCH_QUERY_OPTIONAL_COUNT, 2);
}

#[test]
fn table_patch_derive_emits_table_name() {
    assert_eq!(UserTablePatch::__JOLTR_PATCH_QUERY_TABLE_NAME, "users");
}

#[test]
fn empty_patch_missing_table_returns_none() {
    assert!(EmptyPatch::__JOLTR_PATCH_QUERY_TABLE_NAME.is_none());
}

// ── JOLTR-RS-114: to_patch_query compiles and returns correct shape ──

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

// ── JOLTR-RS-116: $N parameter notation + no value interpolation ──

#[test]
fn to_patch_query_uses_dollar_n_placeholders() {
    let patch = AccountPatch {
        display_name: Optional::Some("Alice".to_string()),
        bio: "Engineer".to_string(),
        follower_count: 0,
    };
    let (sql, params) = patch.to_patch_query("id", &99u64);

    assert!(sql.contains("$1"), "SET clause must use $1 placeholder, got: {sql}");
    assert!(sql.contains("$2"), "SET clause must use $2 placeholder, got: {sql}");
    assert!(sql.contains("$3"), "SET clause must use $3 placeholder, got: {sql}");
    assert!(sql.contains("$4"), "WHERE clause must use $4 placeholder, got: {sql}");
    assert_eq!(params.len(), 4, "params must match $N placeholder count");
}

#[test]
fn to_patch_query_never_interpolates_values_into_sql() {
    let patch = AccountPatch {
        display_name: Optional::Some("Alice".to_string()),
        bio: "secret-bio-value".to_string(),
        follower_count: 0,
    };
    let (sql, _params) = patch.to_patch_query("id", &99u64);

    assert!(
        !sql.contains("secret-bio-value"),
        "SQL must not contain direct string value, got: {sql}"
    );
    assert!(
        !sql.contains("Alice"),
        "SQL must not contain direct string value, got: {sql}"
    );
}

// ── JOLTR-RS-117: closing test bundle ──

#[test]
fn to_patch_query_all_fields_some() {
    let patch = AllOptionalPatch {
        title: Optional::Some("Title".to_string()),
        description: Optional::Some("Description".to_string()),
        tags: Optional::Some("a,b".to_string()),
        url: Optional::Some("https://example.com".to_string()),
    };
    let (sql, params) = patch.to_patch_query("id", &99u64);

    for col in &["title", "description", "tags", "url"] {
        assert!(
            sql.contains(col),
            "all-Some field {} must appear in SET clause, got: {sql}",
            col
        );
    }
    assert!(sql.contains("$1"), "first param should be $1, got: {sql}");
    assert!(sql.contains("$2"), "second param should be $2, got: {sql}");
    assert!(sql.contains("$3"), "third param should be $3, got: {sql}");
    assert!(sql.contains("$4"), "fourth param should be $4, got: {sql}");
    assert!(sql.contains("$5"), "WHERE clause should be $5, got: {sql}");
    assert!(!sql.contains("NULL"), "all-Some: no NULL columns expected, got: {sql}");
    assert_eq!(params.len(), 5, "expected 5 params (4 SET + 1 WHERE), got: {sql}");
}

#[test]
fn to_patch_query_all_fields_null() {
    let patch = AllOptionalPatch {
        title: Optional::Null,
        description: Optional::Null,
        tags: Optional::Null,
        url: Optional::Null,
    };
    let (sql, params) = patch.to_patch_query("id", &1u64);

    for col in &["title", "description", "tags", "url"] {
        assert!(
            sql.contains(&format!("{col} = NULL")),
            "all-Null field {} must produce SET col = NULL, got: {sql}",
            col
        );
    }
    assert!(
        sql.contains("$1"),
        "WHERE id param must be $1 (no SET params for all-Null), got: {sql}"
    );
    assert!(
        !sql.contains("$2"),
        "no $2 expected when all fields are Null, got: {sql}"
    );
    assert_eq!(params.len(), 1, "only the id_value param (no SET params for all-Null), got: {sql}");
}

#[test]
fn to_patch_query_mixed_some_null_not_provided() {
    // name=Some("Alpha"), color=Null, size=NotProvided, weight=Some("2kg")
    let patch = MixedStatePatch {
        name: Optional::Some("Alpha".to_string()),
        color: Optional::Null,
        size: Optional::NotProvided,
        weight: Optional::Some("2kg".to_string()),
    };
    let (sql, params) = patch.to_patch_query("item_id", &42u64);

    assert!(
        sql.contains("name = $1"),
        "Some field must produce col = $N, got: {sql}"
    );
    assert!(
        sql.contains("color = NULL"),
        "Null field must produce col = NULL, got: {sql}"
    );
    assert!(
        !sql.contains("size"),
        "NotProvided field must be absent from SET clause, got: {sql}"
    );
    assert!(
        sql.contains("weight = $2"),
        "second Some field must produce col = $2, got: {sql}"
    );
    assert!(
        sql.contains("$3"),
        "WHERE clause must be $3 (after 2 Some params), got: {sql}"
    );
    assert_eq!(params.len(), 3, "expected 3 params (2 SET + 1 WHERE), got: {sql}");
}

#[test]
fn to_patch_query_empty_struct_with_table_returns_noop() {
    let patch = EmptyWithTablePatch;
    let (sql, params) = patch.to_patch_query("id", &7u64);

    assert!(
        sql.contains("no fields to update"),
        "empty-struct patch must return no-fields message, got: {sql}"
    );
    assert_eq!(
        params.len(), 0,
        "empty struct with no SET fields must have zero params (id_value was never pushed), got: {sql}"
    );
}

#[test]
fn to_patch_query_table_name_round_trips_in_sql() {
    let patch = CustomersPatch {
        email: Optional::Some("alice@example.com".to_string()),
        name: Optional::Some("Alice".to_string()),
    };
    let (sql, params) = patch.to_patch_query("customer_id", &1u64);

    assert!(
        sql.contains("UPDATE customers"),
        "SQL must reference the #[patch(\"customers\")] table name, got: {sql}"
    );
    assert!(sql.contains("$1"), "email must be $1, got: {sql}");
    assert!(sql.contains("$2"), "name must be $2, got: {sql}");
    assert!(sql.contains("$3"), "WHERE clause must be $3, got: {sql}");
    assert_eq!(params.len(), 3, "expected 3 params (2 SET + 1 WHERE), got: {sql}");

    assert!(
        !sql.contains("customers@example.com") && !sql.contains("Alice"),
        "string values must NOT be interpolated into SQL, got: {sql}"
    );
}
