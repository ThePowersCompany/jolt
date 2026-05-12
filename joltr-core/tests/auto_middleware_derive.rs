//! JOLTR-RS-046 PRD-mandated integration test.
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
//! The hidden `__JOLTR_AUTO_MIDDLEWARE_FIELD_COUNT` const emitted by the 046
//! derive is the observable witness that parsing succeeded. Later phase10/11
//! items (047-053) replace the marker const with the real `tower::Layer` impl
//! and per-derive extraction helper; the consts coexist with the new surfaces
//! until JOLTR-RS-054's runtime-witness test makes them redundant.
//!
//! As of JOLTR-RS-053, `CreateUserRequest` carries `#[derive(serde::Deserialize)]`
//! so the per-derive `__jolt_extract_from` helper that the macro now emits
//! (which calls `__req.json::<CreateUserRequest>()` for the `body` field of
//! `MixedMiddleware` and `CorsEnabledMiddleware`) compiles.

use joltr_core::{AutoMiddleware, Method, QueryParams, Request};
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

/// JOLTR-RS-050: a struct that opts into the CORS layer via the helper
/// `#[cors]` attribute. The integration test verifies the
/// `attributes(cors)` opt-in on the derive (so rustc accepts the attribute at
/// the source site) AND that the parsed flag flows through to the
/// `__JOLTR_AUTO_MIDDLEWARE_CORS` marker const.
#[derive(AutoMiddleware)]
#[cors]
#[allow(dead_code)]
struct CorsEnabledMiddleware {
    body: CreateUserRequest,
}

/// JOLTR-RS-053: a middleware struct that exercises the per-derive
/// `__jolt_extract_from(&Request) -> Self` helper end-to-end. Mixes the three
/// extraction shapes the PRD-mandates: body (JSON deserialization), query
/// (HashMap clone), and req (by-value clone). The runtime test below builds a
/// fake `joltr_core::Request`, calls the helper, and asserts each field was
/// populated correctly.
#[derive(AutoMiddleware)]
#[allow(dead_code)]
struct ExtractMw {
    body: CreateUserRequest,
    query_params: HashMap<String, String>,
    req: Request,
}

#[derive(AutoMiddleware)]
#[allow(dead_code)]
struct TypedQueryMw {
    query: QueryParams<Filters>,
}

#[derive(AutoMiddleware)]
#[allow(dead_code)]
struct BorrowRequestMw<'a> {
    req: &'a Request,
}

/// JOLTR-RS-053: derives `serde::Deserialize` so the auto-middleware extraction
/// helper's body-extraction call (`__req.json::<CreateUserRequest>()`) compiles.
/// Before 053 this only had to be a syntactic placeholder; the macro now emits
/// real deserialization codegen for any `body: T` field.
#[derive(serde::Deserialize)]
#[allow(dead_code)]
struct CreateUserRequest {
    name: String,
    age: u32,
}

#[derive(Debug, serde::Deserialize, PartialEq, Eq)]
struct Filters {
    page: u32,
    filter: String,
}

#[test]
fn unit_middleware_derive_emits_zero_field_count() {
    assert_eq!(UnitMiddleware::__JOLTR_AUTO_MIDDLEWARE_FIELD_COUNT, 0);
}

#[test]
fn mixed_middleware_derive_emits_correct_field_count() {
    // Six fields declared above → derive must report exactly six. A regression
    // that mis-counted (e.g. by skipping a field with a non-trivial generic
    // path or by dropping the trailing field) would surface here.
    assert_eq!(MixedMiddleware::__JOLTR_AUTO_MIDDLEWARE_FIELD_COUNT, 6);
}

#[test]
fn middleware_without_cors_attribute_emits_cors_false() {
    // JOLTR-RS-050: a struct WITHOUT the `#[cors]` attribute has the cors
    // marker const set to false. Both `UnitMiddleware` and `MixedMiddleware`
    // exercise this — neither carries `#[cors]`. Wrapped in `const { ... }`
    // so the const-value comparison happens at compile time (a regression
    // that emitted `true` here would fail to build the test binary).
    const { assert!(!UnitMiddleware::__JOLTR_AUTO_MIDDLEWARE_CORS) }
    const { assert!(!MixedMiddleware::__JOLTR_AUTO_MIDDLEWARE_CORS) }
}

#[test]
fn middleware_with_cors_attribute_emits_cors_true() {
    // JOLTR-RS-050: a struct WITH the `#[cors]` attribute has the cors marker
    // const set to true. The `CorsEnabledMiddleware` declaration above is the
    // source-site witness that `#[cors]` is accepted by rustc (via the
    // derive's `attributes(cors)` opt-in); the const-block assertion is the
    // parse-witness that the derive observed the attribute and propagated it
    // to codegen. Same const-block rationale as the false-case test.
    const { assert!(CorsEnabledMiddleware::__JOLTR_AUTO_MIDDLEWARE_CORS) }
    // The field-count const still works alongside the cors const.
    assert_eq!(
        CorsEnabledMiddleware::__JOLTR_AUTO_MIDDLEWARE_FIELD_COUNT,
        1
    );
}

// JOLTR-RS-051 PRD verification: "Generated code compiles and implements
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
// per-request-construction concern is JOLTR-RS-053's, not 051's).
fn _assert_implements_tower_layer<L, S>()
where
    L: joltr_core::tower::Layer<S>,
{
}

// Force monomorphization at link time so the trait bound MUST resolve. If a
// future regression breaks the impl, these `const _` blocks fail to compile
// with a "the trait `tower::Layer<()>` is not implemented for ..." diagnostic
// pointing at the specific middleware struct.
const _: fn() = _assert_implements_tower_layer::<UnitMiddleware, ()>;
const _: fn() = _assert_implements_tower_layer::<MixedMiddleware, ()>;
const _: fn() = _assert_implements_tower_layer::<CorsEnabledMiddleware, ()>;

// JOLTR-RS-051: the `Layer::Service` associated type points at the generated
// wrapper, which itself implements `tower::Service<Req>` whenever the inner
// service does. Pinning this on a concrete inner-service shape proves the
// wrapper's bound chain resolves end-to-end (not just the Layer impl in
// isolation), and the `.call(...)`-then-`.await` flow exercises the
// delegation that JOLTR-RS-052 will wrap and JOLTR-RS-053 will splice
// extraction into.
//
// `tower::service_fn` produces a `Service<Req>` from any closure
// `Fn(Req) -> Future<Output = Result<Resp, Err>>`. We use the simplest
// possible inner — `Fn(()) -> Ready<Result<(), Infallible>>` — to avoid
// needing real HTTP types here.
#[tokio::test]
async fn middleware_layer_wraps_inner_service() {
    use joltr_core::tower::{Layer, Service};
    use std::convert::Infallible;
    use std::future::ready;

    let inner = joltr_core::tower::service_fn(|_: ()| ready(Ok::<_, Infallible>(())));
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

// JOLTR-RS-053 PRD verification: "Generated call() extracts body into struct,
// query into struct, req ref into struct field."
//
// 053 emits a per-derive `__jolt_extract_from(&Request) -> Self` helper rather
// than splicing the extraction calls directly into the wrapper service's
// `call()` body. The wrapper is generic over `__Req` (per JOLTR-RS-051), so
// inlining `__req.json::<T>()` would force `__Req: ::joltr_core::Request`
// (breaking the `Service<()>` test above). The helper is the standalone
// observable surface this test exercises end-to-end: build a fake
// `joltr_core::Request`, call the helper, assert per-field population.
//
// The chain markers in the wrapper's `call()` body stay as marker statements
// at 053; replacing them with calls into `__jolt_extract_from` is whichever
// PRD lands the JoltR-aware tower layer (likely after JOLTR-RS-055-058's CORS
// middleware finishes the layer-design loop).
#[test]
fn extract_from_populates_body_query_request_fields() {
    use axum::http::HeaderMap;

    // Synthesize an inbound request the way Router::build_jolt_request would,
    // but inline so the test doesn't depend on the dispatch path. The body is
    // a JSON byte string the auto-middleware will deserialize via
    // serde_json::from_slice — the same path Request::json takes.
    let req = Request {
        method: Method::Post,
        path: "/users".to_string(),
        headers: HeaderMap::new(),
        query_params: HashMap::from([
            ("page".to_string(), "2".to_string()),
            ("filter".to_string(), "active".to_string()),
        ]),
        body: br#"{"name":"alice","age":30}"#.to_vec(),
        cookies: vec![],
        finished: false,
    };

    let mw = ExtractMw::__jolt_extract_from(&req);

    // Body: deserialized via serde_json into the typed shape declared on the
    // struct. Pinning both fields confirms the macro's `__req.json::<T>()`
    // call wired up the right T (a regression that emitted `<()>::json` would
    // surface as a deserialize error here).
    assert_eq!(mw.body.name, "alice");
    assert_eq!(mw.body.age, 30);

    // Query params: HashMap-shape extraction is a clone of the request's
    // populated `query_params`. A regression that returned the empty default
    // would fail this assertion.
    assert_eq!(mw.query_params.len(), 2);
    assert_eq!(mw.query_params.get("page").map(String::as_str), Some("2"));
    assert_eq!(
        mw.query_params.get("filter").map(String::as_str),
        Some("active")
    );

    // Request injection (by-value): the helper clones the active request into
    // the field. Asserting `path` confirms the clone preserved the request
    // contents end-to-end (path is the most distinctive field on the request
    // we built above).
    assert_eq!(mw.req.path, "/users");
    assert_eq!(mw.req.method, Method::Post);
}

#[test]
fn extract_from_populates_typed_query_params_field() {
    use axum::http::HeaderMap;

    let req = Request {
        method: Method::Get,
        path: "/users".to_string(),
        headers: HeaderMap::new(),
        query_params: HashMap::from([
            ("page".to_string(), "2".to_string()),
            ("filter".to_string(), "active".to_string()),
        ]),
        body: Vec::new(),
        cookies: vec![],
        finished: false,
    };

    let mw = TypedQueryMw::__jolt_extract_from(&req);

    assert_eq!(mw.query.page, 2);
    assert_eq!(mw.query.filter, "active");
}

#[test]
fn extract_from_populates_by_ref_request_field() {
    use axum::http::HeaderMap;

    let req = Request {
        method: Method::Get,
        path: "/borrowed".to_string(),
        headers: HeaderMap::new(),
        query_params: HashMap::new(),
        body: Vec::new(),
        cookies: vec![],
        finished: false,
    };

    let mw = BorrowRequestMw::__jolt_extract_from(&req);

    assert!(std::ptr::eq(mw.req, &req));
    assert_eq!(mw.req.path, "/borrowed");
    assert_eq!(mw.req.method, Method::Get);
}

#[tokio::test]
async fn middleware_layer_returns_bad_request_for_invalid_typed_query() {
    use axum::body::{to_bytes, Body};
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::Response as AxumResponse;
    use joltr_core::tower::{Layer, Service};
    use std::convert::Infallible;

    let mw = TypedQueryMw {
        query: QueryParams(Filters {
            page: 0,
            filter: String::new(),
        }),
    };
    let inner = joltr_core::tower::service_fn(|_: Request| async move {
        panic!("inner service must not run when typed query extraction fails");
        #[allow(unreachable_code)]
        Ok::<_, Infallible>(AxumResponse::new(Body::empty()))
    });
    let mut wrapped = mw.layer(inner);

    let bad_req = Request {
        method: Method::Get,
        path: "/users".to_string(),
        headers: HeaderMap::new(),
        query_params: HashMap::from([
            ("page".to_string(), "not-a-number".to_string()),
            ("filter".to_string(), "active".to_string()),
        ]),
        body: Vec::new(),
        cookies: vec![],
        finished: false,
    };

    let response = <_ as Service<Request>>::call(&mut wrapped, bad_req)
        .await
        .expect("middleware returns a response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), 1024)
        .await
        .expect("body collects");
    let body = std::str::from_utf8(&body).expect("body is utf-8");
    assert!(
        body.contains("Invalid query parameter"),
        "body must include query parse error, got: {body}"
    );
}

#[tokio::test]
async fn middleware_layer_returns_finished_request_ext_response() {
    use axum::body::{to_bytes, Body};
    use axum::extract::Request as AxumRequest;
    use axum::http::StatusCode;
    use axum::response::Response as AxumResponse;
    use joltr_core::tower::{Layer, Service};
    use joltr_core::RequestExt;
    use std::convert::Infallible;
    use std::sync::Arc;

    let mw = TypedQueryMw {
        query: QueryParams(Filters {
            page: 0,
            filter: String::new(),
        }),
    };
    let inner = joltr_core::tower::service_fn(|_: AxumRequest| async move {
        panic!("inner service must not run when RequestExt is finished");
        #[allow(unreachable_code)]
        Ok::<_, Infallible>(AxumResponse::new(Body::empty()))
    });
    let mut wrapped = mw.layer(inner);

    let ext = Arc::new(RequestExt::new());
    ext.set_response(
        AxumResponse::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Body::from("auth required"))
            .unwrap(),
    );
    ext.mark_finished();

    let mut req = AxumRequest::builder()
        .method("GET")
        .uri("/users?page=2&filter=active")
        .body(Body::empty())
        .unwrap();
    req.extensions_mut().insert(Arc::clone(&ext));

    let response = <_ as Service<AxumRequest>>::call(&mut wrapped, req)
        .await
        .expect("middleware returns the stashed response");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = to_bytes(response.into_body(), 1024)
        .await
        .expect("body collects");
    assert_eq!(&body[..], b"auth required");
}

#[test]
#[should_panic(expected = "body deserialization failed")]
fn middleware_layer_runs_extraction_before_inner_service() {
    use axum::http::HeaderMap;
    use joltr_core::tower::{Layer, Service};
    use std::convert::Infallible;
    use std::future::ready;

    let dummy_req = Request {
        method: Method::Get,
        path: "/dummy".to_string(),
        headers: HeaderMap::new(),
        query_params: HashMap::new(),
        body: Vec::new(),
        cookies: vec![],
        finished: false,
    };
    let mw = ExtractMw {
        body: CreateUserRequest {
            name: String::new(),
            age: 0,
        },
        query_params: HashMap::new(),
        req: dummy_req,
    };
    let inner = joltr_core::tower::service_fn(|_: Request| {
        panic!("inner service must not run when extraction rejects the request body");
        #[allow(unreachable_code)]
        ready(Ok::<_, Infallible>(axum::response::Response::new(
            axum::body::Body::empty(),
        )))
    });
    let mut wrapped = mw.layer(inner);

    let bad_req = Request {
        method: Method::Post,
        path: "/users".to_string(),
        headers: HeaderMap::new(),
        query_params: HashMap::new(),
        body: b"{not valid json".to_vec(),
        cookies: vec![],
        finished: false,
    };

    let _ = <_ as Service<Request>>::call(&mut wrapped, bad_req);
}
