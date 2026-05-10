//! [`tower::Service`] wrapper that attaches a fresh [`RequestExt`] to every
//! inbound request and dispatches to either an [`axum::Router`] (legacy
//! pass-through, kept for tests + composition) or an [`EndpointRegistry`]
//! (registry-driven dispatch — JOLT-RS-034).
//!
//! `RequestExt` carries the `finished` latch that downstream middleware uses
//! to short-circuit handler dispatch (JOLT-RS-035). Injecting it at the
//! outermost service layer guarantees every handler that runs through this
//! Router has the latch reachable via `req.extensions().get::<Arc<RequestExt>>()`,
//! regardless of which dispatch backend is in use.
//!
//! Ladder for phase07:
//! - JOLT-RS-033 (landed): `Service` impl + `RequestExt` injection.
//! - JOLT-RS-034 (this file): `from_registry` + registry-driven `call`.
//! - JOLT-RS-035 (next): finished-flag short-circuit between middleware and
//!   handler dispatch.
//! - JOLT-RS-036 (next): `Router::new(registry)` as the canonical constructor;
//!   likely collapses [`Inner`] to a single variant once `from_axum` is
//!   retired.
//! - JOLT-RS-037 (next): full test sweep including 405 method-mismatch
//!   refinement (this file's 034 dispatch returns 404 for both unknown paths
//!   and method mismatches, matching the JOLT-RS-034 step text verbatim).
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

use axum::body::Body;
use axum::extract::Request as AxumRequest;
use axum::http::StatusCode;
use axum::response::Response;
use tower::Service;

use crate::endpoint_registry::{build_jolt_request, EndpointRegistry};
use crate::method::Method;
use crate::request_ext::RequestExt;

/// Outermost `tower::Service` for a Jolt server. Dispatches inbound requests
/// to either a wrapped [`axum::Router`] or an [`EndpointRegistry`] (walked in
/// iteration order, longest-path-first courtesy of [`EndpointRegistry::sort`]).
#[derive(Clone)]
pub struct Router {
    inner: Inner,
}

#[derive(Clone)]
enum Inner {
    Axum(axum::Router),
    Registry(Arc<EndpointRegistry>),
}

impl Router {
    /// Wrap an existing [`axum::Router`] (typically the one produced by
    /// [`EndpointRegistry::build_router`]).
    ///
    /// Kept as a lower-level entry point for callers that already hold an
    /// `axum::Router` (e.g. tests, future composition helpers). JOLT-RS-036
    /// will land `Router::new(registry)` as the primary registry-based
    /// constructor.
    pub fn from_axum(inner: axum::Router) -> Self {
        Self {
            inner: Inner::Axum(inner),
        }
    }

    /// Build a Router that dispatches via registry walk: iterate the registry
    /// in its current order looking for the first endpoint whose `(path,
    /// method)` matches the inbound request, and call that endpoint's handler.
    /// On no match, respond with `404 Not Found`.
    ///
    /// The registry is sorted longest-path-first at construction so that
    /// `/api/hello` is preferred over `/api`. JOLT-RS-036 will rename this to
    /// `Router::new(registry)`; the current name disambiguates from the
    /// reserved [`Self::from_axum`] entry while phase07 is still in flight.
    pub fn from_registry(mut registry: EndpointRegistry) -> Self {
        registry.sort();
        Self {
            inner: Inner::Registry(Arc::new(registry)),
        }
    }
}

impl Service<AxumRequest> for Router {
    type Response = Response;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response, Infallible>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        match &mut self.inner {
            Inner::Axum(router) => {
                <axum::Router as Service<AxumRequest>>::poll_ready(router, cx)
            }
            Inner::Registry(_) => Poll::Ready(Ok(())),
        }
    }

    fn call(&mut self, mut req: AxumRequest) -> Self::Future {
        req.extensions_mut().insert(Arc::new(RequestExt::new()));
        match &mut self.inner {
            Inner::Axum(router) => {
                let mut inner = router.clone();
                Box::pin(async move { inner.call(req).await })
            }
            Inner::Registry(registry) => {
                let registry = Arc::clone(registry);
                Box::pin(async move {
                    // Unparseable HTTP verbs (e.g. CONNECT/TRACE) cannot match
                    // any registered endpoint, so they short-circuit to 404
                    // before allocating the Jolt request snapshot.
                    let Ok(method) = req.method().as_str().parse::<Method>() else {
                        return Ok(not_found());
                    };
                    let path = req.uri().path().to_string();
                    for endpoint in registry.iter() {
                        if endpoint.method() == method && endpoint.path() == path {
                            let jolt_req = build_jolt_request(req).await;
                            return Ok(endpoint.handler(jolt_req).await);
                        }
                    }
                    Ok(not_found())
                })
            }
        }
    }
}

fn not_found() -> Response {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::empty())
        .expect("static 404 builder always succeeds")
}
