//! Body parsing `tower::Layer` (JOLT-RS-059).
//!
//! [`ParseBodyLayer<T>`] buffers an inbound axum request's body bytes once,
//! restores the buffered bytes onto the request so downstream services
//! (notably [`Router`](crate::Router)'s registry-driven body re-read) can
//! continue to consume them, and attempts `serde_json::from_slice::<T>` on the
//! buffered bytes. On success, the parsed `T` is inserted into the request's
//! extensions so a downstream service (or the AutoMiddleware-derived struct
//! consuming the request) can pull it out with `req.extensions().get::<T>()`.
//!
//! On parse failure, this slice (JOLT-RS-059) leaves the request unchanged
//! and delegates to the inner service. JOLT-RS-060 will replace that
//! pass-through with a `400 Bad Request` short-circuit plus
//! [`RequestExt::mark_finished`](crate::RequestExt::mark_finished) once the
//! typed-error surface lands; pinning the layer shape here keeps that future
//! change additive (a new branch inside `call`, not a rebuilt service).
//!
//! Architectural decisions pinned here for JOLT-RS-060..062 to build on:
//!
//! 1. **Layer is parameterized over the body type `T`**, not over the user's
//!    middleware struct. The macro-driven `__jolt_extract_from(&Request)`
//!    already runs per-field `serde_json::from_slice` (see
//!    `jolt-macros::auto_middleware`); parameterizing the *layer* over the
//!    middleware struct would duplicate that field-shape inspection at
//!    tower-stack assembly time. Parameterizing over `T` keeps the layer
//!    composable: a caller wiring `ParseBodyLayer::<CreateUserRequest>::new()`
//!    onto a `tower::ServiceBuilder` produces a service that hands the parsed
//!    body to whatever downstream consumer wants it.
//!
//! 2. **Parsed `T` lands in request extensions, NOT in `RequestExt`.**
//!    [`RequestExt`](crate::RequestExt) is for cross-layer control flow
//!    (the `finished` latch, stashed responses). Parsed body content is
//!    request-scoped *data*, not control state — extensions are the right
//!    home (matches axum's `Extension<T>` convention and lets a handler reach
//!    the value via the standard extension-getter API). JOLT-RS-061's `String`
//!    body extraction will use the same channel.
//!
//! 3. **Body bytes are restored onto the request after buffering.**
//!    [`axum::body::to_bytes`] consumes the body. Downstream Jolt path
//!    (`router::Router::call` → `endpoint_registry::build_jolt_request`)
//!    re-buffers the body when building the Jolt [`Request`](crate::Request)
//!    snapshot, so the layer reconstitutes the body via `Body::from(bytes)`
//!    rather than draining it. The double-buffer is acceptable for the
//!    architectural slice; JOLT-RS-062+ can collapse to a single buffer by
//!    stashing a `BufferedBody(Bytes)` extension and teaching
//!    `build_jolt_request` to consume it.
//!
//! 4. **Parse failure is silent in 059.** The layer attempts the parse, and
//!    on `Err` simply does not insert into extensions. The downstream
//!    AutoMiddleware codegen's `.expect(...)` panics today on the same
//!    failure (see the `JOLT-RS-062 will replace this panic` note in
//!    `jolt-macros::auto_middleware::field_init_expr`). JOLT-RS-060 will move
//!    the failure-rejection contract into THIS layer (return 400 + mark
//!    finished), at which point the codegen's `.expect` becomes unreachable
//!    in the wired-server path.
//!
//! 5. **Body cap mirrors [`build_jolt_request`].** The `u32::MAX` ceiling is
//!    a safety valve, not policy — the same temporary cap that Router uses.
//!    A future PRD item (likely 062 or later) can replace both call sites
//!    with a configurable limit.
//!
//! [`Router`]: crate::Router
//! [`RequestExt`]: crate::RequestExt

use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::body::{Body, Bytes};
use axum::extract::Request as AxumRequest;
use axum::response::Response;
use serde::de::DeserializeOwned;
use tower::{Layer, Service};

/// `tower::Layer` that buffers the request body and attempts to deserialize
/// it as `T`. See module docs for the architectural contract (extension
/// channel, body-restoration policy, silent-failure-in-059 stance).
///
/// `T` is captured as a [`PhantomData`] over `fn() -> T` so the layer is
/// `Send + Sync` regardless of whether `T` itself is. The `fn() -> T` shape
/// (variance: covariant in `T`) matches the conventional zero-sized-type
/// marker for "produces values of `T`" without imposing auto-trait bounds.
pub struct ParseBodyLayer<T> {
    _marker: PhantomData<fn() -> T>,
}

impl<T> ParseBodyLayer<T> {
    /// Construct a parser layer for body type `T`. The layer carries no
    /// runtime state, so a fresh layer is functionally identical to any
    /// other for the same `T`.
    pub fn new() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

impl<T> Default for ParseBodyLayer<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Clone for ParseBodyLayer<T> {
    // Manually implemented — `#[derive(Clone)]` would require `T: Clone` even
    // though `T` only appears under `PhantomData<fn() -> T>` and isn't a
    // runtime field. The standard ServiceBuilder composition path requires
    // Clone on the layer, so the manual impl is load-bearing.
    fn clone(&self) -> Self {
        Self::new()
    }
}

impl<T> std::fmt::Debug for ParseBodyLayer<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParseBodyLayer").finish()
    }
}

impl<S, T> Layer<S> for ParseBodyLayer<T> {
    type Service = ParseBodyService<S, T>;

    fn layer(&self, inner: S) -> Self::Service {
        ParseBodyService {
            inner,
            _marker: PhantomData,
        }
    }
}

/// Inner-service wrapper produced by [`ParseBodyLayer::layer`]. Buffers the
/// request body and inserts a parsed `T` into request extensions on success.
///
/// On parse failure (JOLT-RS-059 contract): the layer leaves the request's
/// extensions untouched and delegates to the inner service. JOLT-RS-060 will
/// replace the failure branch with a `400 Bad Request` short-circuit; the
/// service shape pinned here makes that change additive.
pub struct ParseBodyService<S, T> {
    inner: S,
    _marker: PhantomData<fn() -> T>,
}

impl<S: Clone, T> Clone for ParseBodyService<S, T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _marker: PhantomData,
        }
    }
}

impl<S: std::fmt::Debug, T> std::fmt::Debug for ParseBodyService<S, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParseBodyService")
            .field("inner", &self.inner)
            .finish()
    }
}

impl<S, T> Service<AxumRequest> for ParseBodyService<S, T>
where
    S: Service<AxumRequest, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
    T: DeserializeOwned + Clone + Send + Sync + 'static,
{
    type Response = Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Response, S::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: AxumRequest) -> Self::Future {
        // Standard tower delegation: poll_ready was driven on `self.inner`,
        // so `call` must use that same instance. Replace it with a clone
        // we DON'T call; the caller's next poll_ready readies that slot.
        // Same pattern as `CorsService::call` (JOLT-RS-056).
        let cloned = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, cloned);
        Box::pin(async move {
            let (parts, body) = req.into_parts();
            let bytes = buffer_body(body).await;

            // Restore the buffered bytes as a fresh Body BEFORE attempting
            // the parse. Even on parse failure we want the inner service to
            // receive a fully-formed request — that keeps the silent-failure
            // contract (point 4 in module docs) compatible with downstream
            // body re-readers (build_jolt_request).
            let mut req = AxumRequest::from_parts(parts, Body::from(bytes.clone()));

            if let Ok(parsed) = serde_json::from_slice::<T>(&bytes) {
                req.extensions_mut().insert(parsed);
            }

            inner.call(req).await
        })
    }
}

/// Drain an axum [`Body`] into [`Bytes`] under the same `u32::MAX` cap that
/// [`build_jolt_request`](crate::endpoint_registry::build_jolt_request) uses,
/// returning an empty buffer on I/O error. Extracted as a helper so future
/// PRD items (configurable limits, single-buffer optimization) have one
/// edit site rather than duplicate logic in the service's `call`.
async fn buffer_body(body: Body) -> Bytes {
    axum::body::to_bytes(body, u32::MAX as usize)
        .await
        .unwrap_or_default()
}
