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

// JOLT-RS-051 PRD verification: "Generated code compiles and implements
// tower::Layer."
//
// The cleanest compile-time witness for "this trait is implemented" is a
// generic free fn whose `where` bound only resolves if the impl actually
// exists. Calling it (or just naming the resolved monomorphization) forces
// the trait-resolution at compile time — a regression that dropped the
// `impl Layer<S> for <Mw>` emission would surface here as a missing-impl
// error rather than silently passing.
//
// The assertion is type-only — no values constructed — so it works even for
// middleware structs whose fields don't have `Default` (the
// per-request-construction concern is JOLT-RS-053's, not 051's).
fn _assert_implements_tower_layer<L, S>()
where
    L: jolt_core::tower::Layer<S>,
{
}

// Force monomorphization at link time so the trait bound MUST resolve. If a
// future regression breaks the impl, these `const _` blocks fail to compile
// with a "the trait `tower::Layer<()>` is not implemented for ..." diagnostic
// pointing at the specific middleware struct.
const _: fn() = _assert_implements_tower_layer::<UnitMiddleware, ()>;
const _: fn() = _assert_implements_tower_layer::<MixedMiddleware, ()>;
const _: fn() = _assert_implements_tower_layer::<CorsEnabledMiddleware, ()>;

// JOLT-RS-051: the `Layer::Service` associated type points at the generated
// wrapper, which itself implements `tower::Service<Req>` whenever the inner
// service does. Pinning this on a concrete inner-service shape proves the
// wrapper's bound chain resolves end-to-end (not just the Layer impl in
// isolation), and the `.call(...)`-then-`.await` flow exercises the
// delegation that JOLT-RS-052 will wrap and JOLT-RS-053 will splice
// extraction into.
//
// `tower::service_fn` produces a `Service<Req>` from any closure
// `Fn(Req) -> Future<Output = Result<Resp, Err>>`. We use the simplest
// possible inner — `Fn(()) -> Ready<Result<(), Infallible>>` — to avoid
// needing real HTTP types here.
#[tokio::test]
async fn middleware_layer_wraps_inner_service() {
    use std::convert::Infallible;
    use std::future::ready;
    use jolt_core::tower::{Layer, Service};

    let inner = jolt_core::tower::service_fn(|_: ()| ready(Ok::<_, Infallible>(())));
    // `UnitMiddleware` is a unit struct — instantiate it directly. This is the
    // outer Layer; calling `.layer(inner)` MUST produce a value whose type is
    // the wrapper service the derive emitted.
    let mw = UnitMiddleware;
    let mut wrapped = mw.layer(inner);

    // The wrapper IS a Service — calling it should delegate to `inner` and
    // return `Ok(())`. A regression that emitted the Layer impl but no
    // Service impl on the wrapper would fail to compile here.
    let result = <_ as Service<()>>::call(&mut wrapped, ()).await;
    assert!(result.is_ok(), "wrapper service must delegate to inner");
}
