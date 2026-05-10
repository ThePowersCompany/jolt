//! JOLT-RS-042 PRD-mandated integration test.
//!
//! Verifies the full endpoint-macro → inventory pipeline:
//! a struct decorated with `#[endpoint("/test")]` and a `#[get]` method shows
//! up in `inventory::iter::<RegisteredEndpoint>()` at static-init time, with
//! the correct path + method.
//!
//! This is an *integration* test (not a unit test) because the macro can
//! only be exercised through cargo's compile pipeline — the proc-macro
//! crate's own unit tests parse-check the emitted token stream but cannot
//! actually expand and link the inventory submission.

use jolt_core::{endpoint, Method, RegisteredEndpoint, Response};

struct Probe;

#[endpoint("/probe")]
#[allow(dead_code)] // handler is not invoked by 042's test surface; 045 wires it in.
impl Probe {
    #[get]
    fn ping(&self) -> Response<()> {
        unimplemented!("043 will generate the wrapper that calls this")
    }

    #[post]
    fn record(&self) -> Response<()> {
        unimplemented!("043 will generate the wrapper that calls this")
    }
}

#[test]
fn endpoint_macro_registers_one_entry_per_verb_method_in_inventory() {
    let entries: Vec<&RegisteredEndpoint> = jolt_core::inventory::iter::<RegisteredEndpoint>
        .into_iter()
        .filter(|e| e.path == "/probe")
        .collect();
    // PRD-mandated check: registration appears at server start.
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
