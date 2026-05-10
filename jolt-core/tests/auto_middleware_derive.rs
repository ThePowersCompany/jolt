//! JOLT-RS-046 PRD-mandated integration test.
//!
//! Verifies that `#[derive(AutoMiddleware)]` compiles on a struct with a
//! variety of field types — the PRD's listed verification: "Derive compiles
//! on a struct with various field types."
//!
//! This is an integration test (not a unit test) because the derive macro can
//! only be exercised through cargo's compile pipeline. The proc-macro crate's
//! own unit tests parse-check the emitted token stream but cannot expand and
//! type-check the derive against a real `DeriveInput` from a downstream crate.
//!
//! The hidden `__JOLT_AUTO_MIDDLEWARE_FIELD_COUNT` const emitted by the 046
//! derive is the observable witness that parsing succeeded. Later phase10/11
//! items (047-053) replace the marker const with the real `tower::Layer` impl;
//! at that point this test relaxes (or moves) accordingly.

use jolt_core::{AutoMiddleware, Request};
use std::collections::HashMap;

/// Unit-style middleware: zero fields. The derive must accept this and report
/// a field count of 0.
#[derive(AutoMiddleware)]
struct UnitMiddleware;

/// Mixed field types — the PRD-mandated "various field types" surface. The
/// fields cover the type families that 047-049 will key on:
/// - `body: CreateUserRequest` — body-candidate (a custom DeserializeOwned type),
/// - `query_params: HashMap<String, String>` — query-extraction shape,
/// - `headers: HashMap<String, Vec<u8>>` — generic-arg-rich custom shape,
/// - `count: usize` — primitive,
/// - `flag: bool` — primitive,
/// - `req: Option<Request>` — wrapped framework type.
///
/// `CreateUserRequest` is a plain struct in this test file; it does not need
/// to actually implement `DeserializeOwned` for 046 (parsing is purely
/// syntactic — no trait bounds are emitted yet).
#[derive(AutoMiddleware)]
#[allow(dead_code)]
struct MixedMiddleware {
    body: CreateUserRequest,
    query_params: HashMap<String, String>,
    headers: HashMap<String, Vec<u8>>,
    count: usize,
    flag: bool,
    req: Option<Request>,
}

/// JOLT-RS-050: a struct that opts into the CORS layer via the helper
/// `#[cors]` attribute. The integration test verifies the
/// `attributes(cors)` opt-in on the derive (so rustc accepts the attribute at
/// the source site) AND that the parsed flag flows through to the
/// `__JOLT_AUTO_MIDDLEWARE_CORS` marker const.
#[derive(AutoMiddleware)]
#[cors]
#[allow(dead_code)]
struct CorsEnabledMiddleware {
    body: CreateUserRequest,
}

#[allow(dead_code)]
struct CreateUserRequest {
    name: String,
    age: u32,
}

#[test]
fn unit_middleware_derive_emits_zero_field_count() {
    assert_eq!(UnitMiddleware::__JOLT_AUTO_MIDDLEWARE_FIELD_COUNT, 0);
}

#[test]
fn mixed_middleware_derive_emits_correct_field_count() {
    // Six fields declared above → derive must report exactly six. A regression
    // that mis-counted (e.g. by skipping a field with a non-trivial generic
    // path or by dropping the trailing field) would surface here.
    assert_eq!(MixedMiddleware::__JOLT_AUTO_MIDDLEWARE_FIELD_COUNT, 6);
}

#[test]
fn middleware_without_cors_attribute_emits_cors_false() {
    // JOLT-RS-050: a struct WITHOUT the `#[cors]` attribute has the cors
    // marker const set to false. Both `UnitMiddleware` and `MixedMiddleware`
    // exercise this — neither carries `#[cors]`. Wrapped in `const { ... }`
    // so the const-value comparison happens at compile time (a regression
    // that emitted `true` here would fail to build the test binary).
    const { assert!(!UnitMiddleware::__JOLT_AUTO_MIDDLEWARE_CORS) }
    const { assert!(!MixedMiddleware::__JOLT_AUTO_MIDDLEWARE_CORS) }
}

#[test]
fn middleware_with_cors_attribute_emits_cors_true() {
    // JOLT-RS-050: a struct WITH the `#[cors]` attribute has the cors marker
    // const set to true. The `CorsEnabledMiddleware` declaration above is the
    // source-site witness that `#[cors]` is accepted by rustc (via the
    // derive's `attributes(cors)` opt-in); the const-block assertion is the
    // parse-witness that the derive observed the attribute and propagated it
    // to codegen. Same const-block rationale as the false-case test.
    const { assert!(CorsEnabledMiddleware::__JOLT_AUTO_MIDDLEWARE_CORS) }
    // The field-count const still works alongside the cors const.
    assert_eq!(CorsEnabledMiddleware::__JOLT_AUTO_MIDDLEWARE_FIELD_COUNT, 1);
}
