//! JOLT-RS-110 / JOLT-RS-111 / JOLT-RS-112 PRD-mandated integration tests.
//!
//! Verifies that `#[derive(PatchQuery)]` compiles on a struct with
//! `Optional<T>` fields (110), the struct-level `#[patch("table")]`
//! attribute is parsed and emitted (111), and `Optional<T>` fields are
//! detected and counted (112).
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
//! `Optional<T>` is defined locally in this file because the framework's own
//! `jolt_utils::Optional` tri-state type has not been introduced yet (its
//! introduction is gated on JOLT-RS-055+ per the spec's `Optional<T>` note).
//! At 110 the derive is purely syntactic — it captures field idents and
//! types verbatim regardless of whether `Optional` resolves to the framework
//! type or a user-defined enum, so a local declaration is sufficient to pin
//! the PRD verification.

use jolt_core::PatchQuery;

/// Tri-state stand-in for the framework's eventual `jolt_utils::Optional<T>`.
/// At JOLT-RS-110 the derive doesn't inspect the type beyond capturing it
/// verbatim, so the user-defined enum is interchangeable with the framework
/// type for parse-witness purposes.
#[allow(dead_code)]
enum Optional<T> {
    Some(T),
    Null,
    NotProvided,
}

/// Unit-style patch target: zero fields. The derive must accept this and
/// report a field count of 0. A patch with no updatable columns is degenerate
/// but not malformed at the syntactic level — later slices will surface the
/// SQL error at codegen / execute time.
#[derive(PatchQuery)]
struct EmptyPatch;

/// The PRD-mandated surface: a struct with `Optional<T>` fields. Mixes string
/// and numeric inner types to pin that the derive captures the type verbatim
/// regardless of the inner `T` (a regression that hard-coded `Optional<String>`
/// recognition would still pass on `name`/`email`/`bio` but would mis-handle
/// `age: Optional<u32>`).
#[derive(PatchQuery)]
#[allow(dead_code)]
struct UserPatch {
    name: Optional<String>,
    email: Optional<String>,
    age: Optional<u32>,
    bio: Optional<String>,
}

/// Mixed Optional and non-Optional fields. JOLT-RS-112 will classify the
/// Optional fields as tri-state and leave plain fields alone, but at 110 every
/// field is captured verbatim regardless of optional-ness. Pinning the
/// mixed-field count here protects against a regression that filtered fields
/// at parse time (e.g. dropping non-Optional fields prematurely).
#[derive(PatchQuery)]
#[allow(dead_code)]
struct MixedPatch {
    title: Optional<String>,
    view_count: u64,
    tags: Vec<String>,
    body: Optional<String>,
}

#[test]
fn empty_patch_derive_emits_zero_field_count() {
    assert_eq!(EmptyPatch::__JOLT_PATCH_QUERY_FIELD_COUNT, 0);
}

#[test]
fn user_patch_derive_counts_all_optional_fields() {
    assert_eq!(UserPatch::__JOLT_PATCH_QUERY_FIELD_COUNT, 4);
    // All four fields are Optional<T> → optional count must match field count.
    assert_eq!(UserPatch::__JOLT_PATCH_QUERY_OPTIONAL_COUNT, 4);
}

#[test]
fn mixed_patch_derive_counts_optional_and_plain_fields_together() {
    assert_eq!(MixedPatch::__JOLT_PATCH_QUERY_FIELD_COUNT, 4);
    // Two Optional<T> + two plain fields = total 4, optional 2.
    assert_eq!(MixedPatch::__JOLT_PATCH_QUERY_OPTIONAL_COUNT, 2);
}

// ── JOLT-RS-111: #[patch("table_name")] attribute parsing ──

/// The PRD-mandated surface: a struct with `#[patch("users")]` to verify the
/// table-name attribute is parsed through the derive and emitted as an
/// observable compile-time constant.
#[derive(PatchQuery)]
#[patch("users")]
#[allow(dead_code)]
struct UserTablePatch {
    name: Optional<String>,
    email: Optional<String>,
}

#[test]
fn table_patch_derive_emits_table_name() {
    assert_eq!(UserTablePatch::__JOLT_PATCH_QUERY_TABLE_NAME, "users");
}

#[test]
fn empty_patch_missing_table_returns_none() {
    assert!(EmptyPatch::__JOLT_PATCH_QUERY_TABLE_NAME.is_none());
}
