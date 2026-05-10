//! Storage layer for HTTP endpoints registered with a Jolt server.
//!
//! [`EndpointRegistry`] owns boxed [`Endpoint`] trait objects and is the
//! collection that backs [`crate::JoltServer::endpoint`] (JOLT-RS-026). It
//! grows one more method in JOLT-RS-031 (`build_router`); JOLT-RS-029 landed
//! the struct + `register`, and JOLT-RS-030 added `sort` + `iter`.
//!
//! `Send + Sync` are pinned at the trait-object site (`Box<dyn Endpoint +
//! Send + Sync>`) rather than as supertraits on [`Endpoint`] itself. See
//! `endpoint.rs` for the rationale.

use std::cmp::Reverse;

use crate::endpoint::Endpoint;

#[derive(Default)]
pub struct EndpointRegistry {
    endpoints: Vec<Box<dyn Endpoint + Send + Sync>>,
}

impl EndpointRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Box and store an endpoint. Generic over the concrete type so callers
    /// can write `registry.register(MyEndpoint)` without an explicit
    /// `Box::new`; `Send + Sync + 'static` are required because the
    /// trait-object slot adds those auto-trait bounds.
    pub fn register<E: Endpoint + Send + Sync + 'static>(&mut self, endpoint: E) {
        self.endpoints.push(Box::new(endpoint));
    }

    pub fn len(&self) -> usize {
        self.endpoints.len()
    }

    pub fn is_empty(&self) -> bool {
        self.endpoints.is_empty()
    }

    /// Sort registered endpoints by path length, longest first, so that
    /// JOLT-RS-031's `build_router` dispatches `/api/hello` before `/api`.
    /// Uses a stable sort: equal-length paths keep their insertion order.
    pub fn sort(&mut self) {
        self.endpoints.sort_by_key(|e| Reverse(e.path().len()));
    }

    /// Read-only walk over registered endpoints in current order.
    /// Used by tests to verify [`Self::sort`] and by JOLT-RS-031's
    /// `build_router` to drive route construction.
    pub fn iter(&self) -> impl Iterator<Item = &(dyn Endpoint + Send + Sync)> {
        self.endpoints.iter().map(|b| b.as_ref())
    }
}
