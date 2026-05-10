//! [`tower::Service`] wrapper that ensures every inbound request carries an
//! [`Arc<RequestExt>`] and dispatches to either an [`axum::Router`] (legacy
//! pass-through, kept for tests + composition) or an [`EndpointRegistry`]
//! (registry-driven dispatch — JOLT-RS-034).
//!
//! `RequestExt` carries the `finished` latch that middleware uses to
//! short-circuit handler dispatch (JOLT-RS-035). Sharing the latch across the
//! middleware chain is the whole reason it's atomic, so this Router preserves
//! a caller-supplied `Arc<RequestExt>` if one is already in extensions and
//! only inserts a fresh one when no upstream layer has done so. The
//! registry-driven path then checks `is_finished()` before dispatching to the
//! matched handler — if a layer has already finished the request, Router takes
//! the stashed response (or falls back to a 500) and returns it without
//! invoking the handler.
//!
//! Ladder for phase07:
//! - JOLT-RS-033 (landed): `Service` impl + `RequestExt` injection.
//! - JOLT-RS-034 (landed): `from_registry` + registry-driven `call`.
//! - JOLT-RS-035 (landed): preserve-existing-`RequestExt` contract +
//!   finished-flag short-circuit between dispatch and handler.
//! - JOLT-RS-036 (this file): `Router::new(registry)` as the canonical
//!   constructor. Tower Layer composition is "optional" in the sense that
//!   `Router` itself is a [`tower::Service`], so callers stack layers around
//!   it via [`tower::ServiceBuilder`] when they need to (no Router-side
//!   `.layer()` method is required for the registry's hot path).
//! - JOLT-RS-037 (next): full test sweep including 405 method-mismatch
//!   refinement (this file's 034 dispatch returns 404 for both unknown paths
//!   and method mismatches, matching the JOLT-RS-034 step text verbatim).

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
    /// `/api/hello` is preferred over `/api`. Prefer [`Self::new`], which is
    /// the canonical entry point — `from_registry` is retained as a sibling
    /// for callers that want the explicit name and as the implementation site
    /// the constructor delegates to.
    pub fn from_registry(mut registry: EndpointRegistry) -> Self {
        registry.sort();
        Self {
            inner: Inner::Registry(Arc::new(registry)),
        }
    }

    /// Canonical constructor (JOLT-RS-036): build a registry-driven Router
    /// that's immediately ready to serve as a [`tower::Service`]. Equivalent
    /// to [`Self::from_registry`]; documented as the preferred entry point so
    /// downstream phases (auto-middleware codegen, server wiring) can name a
    /// stable constructor.
    ///
    /// "Optional tower Layer stack" semantics: `Router` is itself a
    /// [`tower::Service`], so callers compose tower layers around it
    /// externally via [`tower::ServiceBuilder`] — no Router-side `.layer()`
    /// method is needed for the registry's hot path.
    ///
    /// ```ignore
    /// let router = Router::new(registry);
    /// let svc = tower::ServiceBuilder::new()
    ///     .layer(/* tower::Layer */)
    ///     .service(router);
    /// ```
    pub fn new(registry: EndpointRegistry) -> Self {
        Self::from_registry(registry)
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
        // JOLT-RS-035: preserve a caller-supplied `Arc<RequestExt>` if one is
        // already in extensions. The latch's whole purpose is shared
        // observability across the middleware chain — overwriting here would
        // strand any `mark_finished()` an outer tower layer has already
        // performed and force the chain to re-derive its own state. When no
        // upstream layer has injected one, we insert a fresh ext so handlers
        // can always rely on the extension being present.
        let request_ext: Arc<RequestExt> = match req.extensions().get::<Arc<RequestExt>>() {
            Some(existing) => Arc::clone(existing),
            None => {
                let fresh = Arc::new(RequestExt::new());
                req.extensions_mut().insert(Arc::clone(&fresh));
                fresh
            }
        };

        match &mut self.inner {
            Inner::Axum(router) => {
                let mut inner = router.clone();
                Box::pin(async move { inner.call(req).await })
            }
            Inner::Registry(registry) => {
                let registry = Arc::clone(registry);
                Box::pin(async move {
                    // JOLT-RS-035: if an upstream middleware has already
                    // finished the request, surface its stashed response (or
                    // a 500 fallback if it forgot to stash one) instead of
                    // walking the registry. Skipping the walk before
                    // `build_jolt_request` also avoids the body-buffering cost
                    // for a request whose outcome is already decided.
                    if request_ext.is_finished() {
                        return Ok(short_circuit_response(&request_ext));
                    }

                    // Unparseable HTTP verbs (e.g. CONNECT/TRACE) cannot match
                    // any registered endpoint, so they short-circuit to 404
                    // before allocating the Jolt request snapshot.
                    let Ok(method) = req.method().as_str().parse::<Method>() else {
                        return Ok(not_found());
                    };
                    let path = req.uri().path().to_string();
                    for endpoint in registry.iter() {
                        if endpoint.method() == method && endpoint.path() == path {
                            // Re-check immediately before dispatching: a
                            // future inner-middleware tier (post-035, pre-046)
                            // may finish the request between the route walk
                            // and the handler invocation.
                            if request_ext.is_finished() {
                                return Ok(short_circuit_response(&request_ext));
                            }
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

/// Build the response surfaced when the `finished` latch is set: the stashed
/// response from [`RequestExt::take_response`] if a middleware provided one,
/// otherwise a defensive 500 so a finishing layer that forgot to stash never
/// silently produces a 200 with whatever default body the caller expected.
fn short_circuit_response(ext: &Arc<RequestExt>) -> Response {
    ext.take_response().unwrap_or_else(|| {
        Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::empty())
            .expect("static 500 builder always succeeds")
    })
}
