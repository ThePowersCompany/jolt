//! [`tower::Service`] wrapper around [`axum::Router`] that attaches a fresh
//! [`RequestExt`] to every inbound request before dispatching to the inner
//! axum router.
//!
//! `RequestExt` carries the `finished` latch that downstream middleware uses
//! to short-circuit handler dispatch (JOLT-RS-035). Injecting it at the
//! outermost service layer guarantees every handler that runs through this
//! Router has the latch reachable via `req.extensions().get::<Arc<RequestExt>>()`,
//! regardless of where the request entered the stack.
//!
//! This file is the JOLT-RS-033 surface only: define `Router` + the Service
//! impl. JOLT-RS-034 will add the registry-driven dispatch path, JOLT-RS-035
//! the finished-flag short-circuit, JOLT-RS-036 the
//! `Router::new(EndpointRegistry)` constructor, and JOLT-RS-037 the broader
//! test sweep.
//!
//! The wrapper holds `Arc<RequestExt>` rather than a bare `RequestExt` because
//! middleware layers downstream of this point clone the request's extensions
//! to spawn parallel work; sharing one atomic `finished` flag across those
//! clones is the whole reason the latch is atomic in the first place.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::extract::Request as AxumRequest;
use axum::response::Response;
use tower::Service;

use crate::request_ext::RequestExt;

/// Outermost `tower::Service` for a Jolt server. Wraps an [`axum::Router`] and
/// attaches a fresh [`Arc<RequestExt>`] extension to every incoming request
/// before forwarding to the inner router.
#[derive(Clone)]
pub struct Router {
    inner: axum::Router,
}

impl Router {
    /// Wrap an existing [`axum::Router`] (typically the one produced by
    /// [`crate::EndpointRegistry::build_router`]).
    ///
    /// JOLT-RS-036 will add `Router::new(registry: EndpointRegistry)` as the
    /// primary constructor; this lower-level entry stays available so callers
    /// that already hold an `axum::Router` (e.g. tests, future composition
    /// helpers) don't need to round-trip through a registry.
    pub fn from_axum(inner: axum::Router) -> Self {
        Self { inner }
    }
}

impl Service<AxumRequest> for Router {
    type Response = Response;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response, Infallible>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        <axum::Router as Service<AxumRequest>>::poll_ready(&mut self.inner, cx)
    }

    fn call(&mut self, mut req: AxumRequest) -> Self::Future {
        req.extensions_mut().insert(Arc::new(RequestExt::new()));
        let mut inner = self.inner.clone();
        Box::pin(async move { inner.call(req).await })
    }
}
