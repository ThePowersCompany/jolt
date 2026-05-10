//! JOLT-RS-042 + JOLT-RS-043 PRD-mandated integration tests.
//!
//! 042: a struct decorated with `#[endpoint("/path")]` and verb-tagged methods
//! shows up in `inventory::iter::<RegisteredEndpoint>()` at static-init time
//! with the correct path + method.
//!
//! 043: the same `#[endpoint(..)]` macro emits one `__jolt_handler_<name>`
//! axum-compatible async wrapper per discovered method on a sibling impl
//! block. Each wrapper takes a `::jolt_core::Request` and returns an
//! `EndpointFuture` that resolves to an `axum::response::Response` whose
//! body / status come from invoking the user's `&self` method on a
//! `Default::default()` instance of the endpoint type.
//!
//! This is an *integration* test (not a unit test) because the macro can
//! only be exercised through cargo's compile pipeline — the proc-macro
//! crate's own unit tests parse-check the emitted token stream but cannot
//! actually expand and link the inventory submission or the wrapper.

use axum::body::to_bytes;
use jolt_core::{endpoint, Method, RegisteredEndpoint, Request, Response, StatusCode};
use std::collections::HashMap;

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
    let entries: Vec<&RegisteredEndpoint> = jolt_core::inventory::iter::<RegisteredEndpoint>
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
