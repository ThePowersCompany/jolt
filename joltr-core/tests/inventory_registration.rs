//! JOLTR-RS-042 + JOLTR-RS-043 + JOLTR-RS-044 PRD-mandated integration tests.
//!
//! 042: a struct decorated with `#[endpoint("/path")]` and verb-tagged methods
//! shows up in `inventory::iter::<RegisteredEndpoint>()` at static-init time
//! with the correct path + method.
//!
//! 043: the same `#[endpoint(..)]` macro emits one `__jolt_handler_<name>`
//! axum-compatible async wrapper per discovered method on a sibling impl
//! block. Each wrapper takes a `::joltr_core::Request` and returns an
//! `EndpointFuture` that resolves to an `axum::response::Response` whose
//! body / status come from invoking the user's `&self` method on a
//! `Default::default()` instance of the endpoint type.
//!
//! 044: every entry collected by `inventory::iter::<RegisteredEndpoint>()`
//! across all linked crates is registered into the [`JoltRServer`]'s
//! [`EndpointRegistry`] when [`JoltRServer::start`] (or its in-process sibling
//! [`JoltRServer::into_router`]) is called. The inventory record carries a
//! `handler: fn(Request) -> EndpointFuture` field pointing at the wrapper from
//! 043, which makes the inventory entry directly usable as an [`Endpoint`]
//! trait object via the `impl Endpoint for &'static RegisteredEndpoint` bridge.
//!
//! This is an *integration* test (not a unit test) because the macro can
//! only be exercised through cargo's compile pipeline — the proc-macro
//! crate's own unit tests parse-check the emitted token stream but cannot
//! actually expand and link the inventory submission or the wrapper. The
//! tests live in `joltr-core/tests/` rather than `joltr-macros/tests/` because
//! the link contract is "downstream crate's `#[endpoint]` submits land in
//! joltr-core's `inventory::collect!` slot" — testing it requires a different
//! crate from joltr-core. This test binary IS that different crate.

use axum::body::{to_bytes, Body};
use axum::http::Request as AxumRequest;
use joltr_core::{
    endpoint, EndpointFuture, JoltRServer, Method, RegisteredEndpoint, Request, Response,
    StatusCode,
};
use std::collections::HashMap;
use tower::ServiceExt;

#[derive(Default)]
struct Probe;

#[endpoint("/probe")]
impl Probe {
    #[get]
    fn ping(&self) -> Response<&'static str> {
        Response::new(StatusCode::Ok, "pong")
    }

    #[post]
    fn record(&self) -> Response<&'static str> {
        Response::new(StatusCode::Created, "recorded")
    }
}

/// Second endpoint type defined in the same test crate as `Probe`. Pinned to
/// prove that JOLTR-RS-044's iteration walks every inventory entry, not just
/// the first emitted struct's submits. A regression that broke after the first
/// match (e.g. `iter().next()` instead of `iter()`) would fail the
/// `into_router_serves_secondary_endpoint` test below.
#[derive(Default)]
struct Secondary;

#[endpoint("/secondary")]
impl Secondary {
    #[get]
    fn fetch(&self) -> Response<&'static str> {
        Response::new(StatusCode::Ok, "secondary-pong")
    }
}

fn empty_request(method: Method, path: &str) -> Request {
    Request {
        method,
        path: path.to_string(),
        headers: Default::default(),
        query_params: HashMap::new(),
        body: Vec::new(),
        cookies: Vec::new(),
        finished: false,
    }
}

#[test]
fn endpoint_macro_registers_one_entry_per_verb_method_in_inventory() {
    // PRD-mandated check (042): registration appears at server start.
    let entries: Vec<&RegisteredEndpoint> = joltr_core::inventory::iter::<RegisteredEndpoint>
        .into_iter()
        .filter(|e| e.path == "/probe")
        .collect();
    assert_eq!(
        entries.len(),
        2,
        "expected 2 entries for /probe (#[get] + #[post]), got {}",
        entries.len()
    );
    let methods: Vec<Method> = entries.iter().map(|e| e.method).collect();
    assert!(methods.contains(&Method::Get), "missing GET entry: {methods:?}");
    assert!(methods.contains(&Method::Post), "missing POST entry: {methods:?}");
}

#[tokio::test]
async fn handler_wrapper_invokes_user_method_and_bridges_to_axum_response() {
    // PRD-mandated check (043): "Handler wrapper compiles and can be called
    // with a Request." Goes one step further than the literal verification by
    // also asserting the wrapper actually delegates to the user's `&self`
    // method (status + body round-trip from Probe::ping → axum::Response).
    let req = empty_request(Method::Get, "/probe");
    let response = Probe::__jolt_handler_ping(req).await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 1024)
        .await
        .expect("body collects");
    assert_eq!(&body_bytes[..], b"pong");
}

#[tokio::test]
async fn handler_wrappers_are_distinct_per_method() {
    // Pin that the macro emits a SEPARATE wrapper per discovered verb method
    // (not a shared dispatcher). Probe::__jolt_handler_record must produce
    // POST's "recorded" body / 201 status, not GET's.
    let req = empty_request(Method::Post, "/probe");
    let response = Probe::__jolt_handler_record(req).await;

    assert_eq!(response.status(), axum::http::StatusCode::CREATED);
    let body_bytes = to_bytes(response.into_body(), 1024)
        .await
        .expect("body collects");
    assert_eq!(&body_bytes[..], b"recorded");
}

#[tokio::test]
async fn registered_endpoint_carries_handler_fn_pointer() {
    // JOLTR-RS-044: the inventory record now carries a `handler: fn(Request) ->
    // EndpointFuture` field, populated by the macro with
    // `<SelfTy>::__jolt_handler_<user_fn>`. Pin that the field is wired by
    // looking up the GET /probe entry, type-checking the field shape via a
    // typed let-binding, and round-tripping the wrapper invocation: the same
    // GET /probe handler must produce the same status + body as
    // `Probe::__jolt_handler_ping` directly. A regression that dropped the
    // field, populated it with the wrong wrapper, or changed the fn signature
    // would fail this test.
    let entry = joltr_core::inventory::iter::<RegisteredEndpoint>
        .into_iter()
        .find(|e| e.path == "/probe" && e.method == Method::Get)
        .expect("GET /probe entry present in inventory");
    let handler: fn(Request) -> EndpointFuture = entry.handler;
    let response = handler(empty_request(Method::Get, "/probe")).await;
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 1024)
        .await
        .expect("body collects");
    assert_eq!(&body_bytes[..], b"pong");
}

#[tokio::test]
async fn into_router_serves_inventory_registered_get_endpoint() {
    // JOLTR-RS-044 PRD-mandated verification: "endpoints defined in multiple
    // crates all register into server." This test crate is NOT joltr-core
    // (joltr-core hosts `inventory::collect!`; this test binary defines the
    // submits via `#[endpoint]`), so seeing the submits register and route
    // through `JoltRServer::into_router` proves the cross-crate path works.
    //
    // GET /probe → 200 + "pong".
    let router = JoltRServer::new().into_router();
    let req = AxumRequest::builder()
        .method("GET")
        .uri("/probe")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(req).await.expect("oneshot succeeds");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 1024)
        .await
        .expect("body collects");
    assert_eq!(&body_bytes[..], b"pong");
}

#[tokio::test]
async fn into_router_serves_inventory_registered_post_endpoint() {
    // Same path (/probe), different verb: confirms the registry-build process
    // distinguishes (Method::Get, "/probe") from (Method::Post, "/probe") and
    // routes each to its own __jolt_handler_<name> wrapper. A regression that
    // collapsed the two submits into a single arm or registered them with the
    // wrong handler swap would fail this test.
    let router = JoltRServer::new().into_router();
    let req = AxumRequest::builder()
        .method("POST")
        .uri("/probe")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(req).await.expect("oneshot succeeds");
    assert_eq!(response.status(), axum::http::StatusCode::CREATED);
    let body_bytes = to_bytes(response.into_body(), 1024)
        .await
        .expect("body collects");
    assert_eq!(&body_bytes[..], b"recorded");
}

#[tokio::test]
async fn into_router_serves_secondary_endpoint() {
    // Two distinct endpoint TYPES (Probe and Secondary) defined in the same
    // crate. Pin that JOLTR-RS-044's iteration registers BOTH — a regression
    // that walked only the first inventory entry, or that confused the
    // `<SelfTy>` binding by re-using `Probe`'s for `Secondary`'s submit, would
    // fail this test.
    let router = JoltRServer::new().into_router();
    let req = AxumRequest::builder()
        .method("GET")
        .uri("/secondary")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(req).await.expect("oneshot succeeds");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 1024)
        .await
        .expect("body collects");
    assert_eq!(&body_bytes[..], b"secondary-pong");
}

#[tokio::test]
async fn build_serving_router_wraps_merged_router_in_trace_layer_without_breaking_responses() {
    // JOLTR-RS-068: `start` must apply `tower_http::trace::TraceLayer` to the
    // merged router so every served request emits a tower-http span. The
    // public `build_serving_router` helper produces the SAME router `start`
    // hands to `axum::serve`, so exercising it via `oneshot` proves the
    // TraceLayer wiring composes cleanly with the existing inventory routes
    // (no panics on layer construction, response status/body unchanged).
    //
    // A regression that dropped the TraceLayer would still pass this test —
    // the assertion is a SHAPE pin, not a behavior pin. The PRD verification
    // for 068 is "Server starts, TraceLayer is in the stack." Type-system
    // proof of "in the stack" comes from the `.layer(TraceLayer::new_for_http())`
    // call in `build_serving_router` itself; this test guards against the
    // adjacent regression where adding the layer accidentally breaks the
    // response path (TraceLayer's `on_response` callback could in principle
    // mangle the body if mis-constructed; `new_for_http()` is the standard
    // form that doesn't, so a passing oneshot pins that we used the right
    // constructor).
    let router = JoltRServer::new().build_serving_router(axum::Router::new());
    let req = AxumRequest::builder()
        .method("GET")
        .uri("/probe")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(req).await.expect("oneshot succeeds");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body_bytes = to_bytes(response.into_body(), 1024)
        .await
        .expect("body collects");
    assert_eq!(&body_bytes[..], b"pong");
}

#[test]
fn collect_inventory_endpoints_populates_registry_without_serving() {
    // Companion to into_router: pin the public `collect_inventory_endpoints`
    // method so advanced users (and tests) can drive registration without
    // building a full router or binding a port. The registry length must
    // match the visible inventory entry count once collection runs.
    let inventory_len = joltr_core::inventory::iter::<RegisteredEndpoint>
        .into_iter()
        .count();
    let server = JoltRServer::new().collect_inventory_endpoints();
    assert_eq!(
        server.registry.len(),
        inventory_len,
        "registry length must match inventory iter count after collection"
    );
}
