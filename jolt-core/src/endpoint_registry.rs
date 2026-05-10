//! Storage layer for HTTP endpoints registered with a Jolt server.
//!
//! [`EndpointRegistry`] owns boxed [`Endpoint`] trait objects and is the
//! collection that backs [`crate::JoltServer::endpoint`] (JOLT-RS-026). It
//! grows two more methods in JOLT-RS-030 (`sort`) and JOLT-RS-031
//! (`build_router`); this task lands the struct + `register` only.
//!
//! `Send + Sync` are pinned at the trait-object site (`Box<dyn Endpoint +
//! Send + Sync>`) rather than as supertraits on [`Endpoint`] itself. See
//! `endpoint.rs` for the rationale.

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
}
