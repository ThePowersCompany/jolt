//! Body parsing `tower::Layer`s (JOLT-RS-059..061).
//!
//! Two sibling layers share the buffer-body-then-stash-into-extensions shape:
//!
//! * [`ParseBodyLayer<T>`] (JOLT-RS-059/060) attempts
//!   `serde_json::from_slice::<T>` on the buffered bytes and stashes the
//!   parsed `T` into request extensions. Parse failure short-circuits with a
//!   `400 Bad Request` carrying `"Invalid JSON: <serde error>"`.
//! * [`ParseBodyStringLayer`] (JOLT-RS-061) decodes the buffered bytes as
//!   UTF-8 and stashes the resulting `String` into request extensions. UTF-8
//!   decode failure short-circuits with a `400 Bad Request` carrying
//!   `"Invalid UTF-8: <utf-8 error>"`. Distinct from `ParseBodyLayer<String>`
//!   on purpose: a `String` body in this framework is a raw `text/plain`
//!   payload, not a JSON string literal. Routing it through
//!   `serde_json::from_slice::<String>` would only accept inputs that are
//!   already quoted JSON (`"hello"`, not `hello`), which is the opposite of
//!   what the user means.
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
//! 4. **Parse failure short-circuits with a `400 Bad Request`** carrying a
//!    `text/plain` body of `"Invalid JSON: <serde error>"` (JOLT-RS-060).
//!    The layer also flips
//!    [`RequestExt::mark_finished`](crate::RequestExt::mark_finished) on the
//!    request's existing [`Arc<RequestExt>`](crate::RequestExt) (or freshly
//!    injects one if no upstream layer has) so any composed observers see the
//!    same finished-flag contract Router relies on for its own short-circuit
//!    path. Because the layer returns the 400 directly from `call()`, the
//!    inner service is never invoked on a failed parse; the `mark_finished`
//!    flip is a pure observability signal — it does NOT round-trip through
//!    Router's stash/take dance.
//!
//!    The downstream AutoMiddleware codegen's `.expect(...)` (see the
//!    `JOLT-RS-062 will replace this panic` note in
//!    `jolt-macros::auto_middleware::field_init_expr`) is therefore unreachable
//!    in the wired-server path: malformed bodies are rejected by THIS layer
//!    before they ever reach the macro-emitted extraction.
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
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::body::{Body, Bytes};
use axum::extract::Request as AxumRequest;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use serde::de::DeserializeOwned;
use tower::{Layer, Service};

use crate::request_ext::RequestExt;

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
/// On parse failure (JOLT-RS-060): the layer short-circuits with a
/// `400 Bad Request` carrying `"Invalid JSON: <serde error>"` as a `text/plain`
/// body, and flips
/// [`RequestExt::mark_finished`](crate::RequestExt::mark_finished) so any
/// composed observers see the same finished-flag contract Router relies on.
/// The inner service is NOT invoked when the parse fails.
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
            let (mut parts, body) = req.into_parts();
            let bytes = buffer_body(body).await;

            // Mirror Router/CorsService's preserve-or-inject contract for
            // `Arc<RequestExt>`: reuse an upstream-supplied ext so a flipped
            // `finished` latch is observable to whoever holds the same Arc;
            // inject a fresh one when no upstream layer has so the
            // mark-finished call on the failure branch is always sound.
            let request_ext: Arc<RequestExt> = match parts.extensions.get::<Arc<RequestExt>>() {
                Some(existing) => Arc::clone(existing),
                None => {
                    let fresh = Arc::new(RequestExt::new());
                    parts.extensions.insert(Arc::clone(&fresh));
                    fresh
                }
            };

            // Restore the buffered bytes as a fresh Body. On the success
            // branch the inner service's downstream body re-readers (notably
            // `build_jolt_request`) keep working; on the failure branch we
            // never reach the inner service at all, but rebuilding the
            // request keeps the parts/body shape consistent with the
            // success path (and avoids a second branch on `req.into_parts`).
            let mut req = AxumRequest::from_parts(parts, Body::from(bytes.clone()));

            match serde_json::from_slice::<T>(&bytes) {
                Ok(parsed) => {
                    req.extensions_mut().insert(parsed);
                    inner.call(req).await
                }
                Err(err) => {
                    // JOLT-RS-060: short-circuit with 400 + mark_finished.
                    // The 400 is returned directly from `call` (not stashed
                    // via `RequestExt::set_response`) because ParseBodyLayer
                    // sits OUTSIDE Router in the typical wiring — Router's
                    // stash/take dance fires only when the registry walk
                    // sees `is_finished()`, which won't happen if the
                    // request never reaches Router. An OUTER layer (e.g.
                    // CorsLayer wrapping ParseBodyService) still sees the
                    // returned 400 as the inner-call result and can layer
                    // its own decoration onto it.
                    request_ext.mark_finished();
                    Ok(bad_request_for_parse_error(&err))
                }
            }
        })
    }
}

/// Build the `400 Bad Request` response surfaced when `serde_json::from_slice`
/// rejects the buffered body bytes (JOLT-RS-060). The body is `text/plain` to
/// match the format of the framework's other ad-hoc error responses (Router's
/// 404/405 paths), and carries `"Invalid JSON: <serde error>"` so the caller
/// gets actionable detail without the layer needing to know what shape `T` is.
fn bad_request_for_parse_error(err: &serde_json::Error) -> Response {
    let body = format!("Invalid JSON: {err}");
    Response::builder()
        .status(StatusCode::BAD_REQUEST)
        .header(
            CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )
        .body(Body::from(body))
        .expect("400 response builder always succeeds with static headers + owned body")
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

/// `tower::Layer` that buffers the request body and decodes it as a UTF-8
/// [`String`] (JOLT-RS-061). On success, the decoded string is inserted into
/// the request's extensions so a downstream service (or the
/// AutoMiddleware-derived struct consuming the request) can pull it out with
/// `req.extensions().get::<String>()`. On UTF-8 decode failure, the layer
/// short-circuits with a `400 Bad Request` carrying
/// `"Invalid UTF-8: <error>"` as a `text/plain` body, mirroring
/// [`ParseBodyLayer`]'s JSON-failure contract.
///
/// Architectural notes:
///
/// * **Distinct from `ParseBodyLayer<String>`.** Routing raw `text/plain`
///   bytes through `serde_json::from_slice::<String>` would require the
///   payload to be a JSON string literal (`"hello"`, with quotes); callers
///   wiring a `String` body field want the raw request payload as UTF-8.
///   Two separate layers keep both surfaces unambiguous and let a caller pick
///   the contract that matches the field type.
/// * **Empty body is valid input.** Empty bytes are valid UTF-8 (the empty
///   string), so the layer inserts `String::new()` into extensions and
///   delegates. Empty-body rejection (if any) is the user's responsibility,
///   matching how the JSON layer treats `null` / `""`.
/// * **Body restoration mirrors [`ParseBodyLayer`].** The buffered bytes are
///   re-armed onto the request before delegating so downstream services
///   (notably `build_jolt_request`'s re-read in the registry path) keep
///   working.
/// * **`Arc<RequestExt>` preserve-or-inject mirrors [`ParseBodyService`] and
///   [`CorsService`](crate::CorsService).** A flipped `finished` latch on
///   UTF-8 failure is observable to whoever holds the same Arc, and the
///   contract holds even when the layer composes outside Router.
#[derive(Default, Clone, Debug)]
pub struct ParseBodyStringLayer;

impl ParseBodyStringLayer {
    /// Construct a string-body parser layer. The layer carries no runtime
    /// state, so a fresh layer is functionally identical to any other.
    pub fn new() -> Self {
        Self
    }
}

impl<S> Layer<S> for ParseBodyStringLayer {
    type Service = ParseBodyStringService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        ParseBodyStringService { inner }
    }
}

/// Inner-service wrapper produced by [`ParseBodyStringLayer::layer`]. Buffers
/// the request body and inserts a decoded [`String`] into request extensions
/// on success; short-circuits with a `400 Bad Request` on UTF-8 decode
/// failure. See [`ParseBodyStringLayer`] for the architectural contract.
#[derive(Clone, Debug)]
pub struct ParseBodyStringService<S> {
    inner: S,
}

impl<S> Service<AxumRequest> for ParseBodyStringService<S>
where
    S: Service<AxumRequest, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
{
    type Response = Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Response, S::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: AxumRequest) -> Self::Future {
        // Same `poll_ready`/`call` swap pattern as `ParseBodyService::call`
        // (JOLT-RS-059) and `CorsService::call` (JOLT-RS-056).
        let cloned = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, cloned);
        Box::pin(async move {
            let (mut parts, body) = req.into_parts();
            let bytes = buffer_body(body).await;

            let request_ext: Arc<RequestExt> = match parts.extensions.get::<Arc<RequestExt>>() {
                Some(existing) => Arc::clone(existing),
                None => {
                    let fresh = Arc::new(RequestExt::new());
                    parts.extensions.insert(Arc::clone(&fresh));
                    fresh
                }
            };

            let mut req = AxumRequest::from_parts(parts, Body::from(bytes.clone()));

            match std::str::from_utf8(&bytes) {
                Ok(text) => {
                    req.extensions_mut().insert(text.to_owned());
                    inner.call(req).await
                }
                Err(err) => {
                    request_ext.mark_finished();
                    Ok(bad_request_for_utf8_error(&err))
                }
            }
        })
    }
}

/// Build the `400 Bad Request` response surfaced when [`std::str::from_utf8`]
/// rejects the buffered body bytes (JOLT-RS-061). The body is `text/plain`,
/// mirroring [`bad_request_for_parse_error`]'s format, and carries
/// `"Invalid UTF-8: <utf-8 error>"` so the caller gets actionable detail
/// (byte index of the invalid sequence) without the layer needing to know
/// what shape the user expected.
fn bad_request_for_utf8_error(err: &std::str::Utf8Error) -> Response {
    let body = format!("Invalid UTF-8: {err}");
    Response::builder()
        .status(StatusCode::BAD_REQUEST)
        .header(
            CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )
        .body(Body::from(body))
        .expect("400 response builder always succeeds with static headers + owned body")
}
