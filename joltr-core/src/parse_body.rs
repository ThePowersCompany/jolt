//! Body parsing `tower::Layer`s (JOLTR-RS-059..062).
//!
//! Two sibling layers share the buffer-body-then-stash-into-extensions shape:
//!
//! * [`ParseBodyLayer<T>`] (JOLTR-RS-059/060) attempts
//!   `serde_json::from_slice::<T>` on the buffered bytes and stashes the
//!   parsed `T` into request extensions. Parse failure short-circuits with a
//!   `400 Bad Request` carrying `"Invalid JSON: <serde error>"`.
//! * [`ParseBodyStringLayer`] (JOLTR-RS-061) decodes the buffered bytes as
//!   UTF-8 and stashes the resulting `String` into request extensions. UTF-8
//!   decode failure short-circuits with a `400 Bad Request` carrying
//!   `"Invalid UTF-8: <utf-8 error>"`. Distinct from `ParseBodyLayer<String>`
//!   on purpose: a `String` body in this framework is a raw `text/plain`
//!   payload, not a JSON string literal. Routing it through
//!   `serde_json::from_slice::<String>` would only accept inputs that are
//!   already quoted JSON (`"hello"`, not `hello`), which is the opposite of
//!   what the user means.
//!
//! Both layers enforce a configurable [`max_body_size`] (default 10 MiB).
//! Bodies exceeding the limit short-circuit with a `413 Payload Too Large`
//! carrying `"Body exceeds maximum allowed size: <N> bytes"` (JOLTR-RS-062).
//!
//! Architectural decisions pinned here for JOLTR-RS-060..062 to build on:
//!
//! 1. **Layer is parameterized over the body type `T`**, not over the user's
//!    middleware struct. The macro-driven `__jolt_extract_from(&Request)`
//!    already runs per-field `serde_json::from_slice` (see
//!    `joltr-macros::auto_middleware`); parameterizing the *layer* over the
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
//!    the value via the standard extension-getter API). JOLTR-RS-061's `String`
//!    body extraction will use the same channel.
//!
//! 3. **Body bytes are restored onto the request after buffering.**
//!    [`axum::body::to_bytes`] consumes the body. Downstream JoltR path
//!    (`router::Router::call` → `endpoint_registry::build_jolt_request`)
//!    re-buffers the body when building the JoltR [`Request`](crate::Request)
//!    snapshot, so the layer reconstitutes the body via `Body::from(bytes)`
//!    rather than draining it. The double-buffer is acceptable for the
//!    architectural slice; a future PRD item can collapse to a single buffer
//!    by stashing a `BufferedBody(Bytes)` extension and teaching
//!    `build_jolt_request` to consume it.
//!
//! 4. **Parse failure short-circuits with a `400 Bad Request`** carrying a
//!    `text/plain` body of `"Invalid JSON: <serde error>"` (JOLTR-RS-060).
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
//!    `JOLTR-RS-062 will replace this panic` note in
//!    `joltr-macros::auto_middleware::field_init_expr`) is therefore unreachable
//!    in the wired-server path: malformed bodies are rejected by THIS layer
//!    before they ever reach the macro-emitted extraction.
//!
//! 5. **Oversized body rejection (JOLTR-RS-062).** Both layers carry a
//!    `max_body_size` (default 10 MiB = 10_485_760 bytes). Bodies exceeding
//!    this limit short-circuit with `413 Payload Too Large` + `mark_finished`,
//!    matching the 400/401 response pattern from parse/auth failures.
//!
//! [`Router`]: crate::Router
//! [`RequestExt`]: crate::RequestExt

use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::body::HttpBody as _;
use axum::body::{Body, Bytes};
use axum::extract::Request as AxumRequest;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use serde::de::DeserializeOwned;
use tower::{Layer, Service};

use crate::request_ext::RequestExt;

/// Default maximum body size in bytes (10 MiB).
pub const DEFAULT_MAX_BODY_SIZE: usize = 10_485_760;

/// `tower::Layer` that buffers the request body and attempts to deserialize
/// it as `T`. See module docs for the architectural contract (extension
/// channel, body-restoration policy, oversized-body rejection).
///
/// `T` is captured as a [`PhantomData`] over `fn() -> T` so the layer is
/// `Send + Sync` regardless of whether `T` itself is. The `fn() -> T` shape
/// (variance: covariant in `T`) matches the conventional zero-sized-type
/// marker for "produces values of `T`" without imposing auto-trait bounds.
///
/// [`max_body_size`] is the only non-ZST runtime field — set via the builder
/// method; defaults to [`DEFAULT_MAX_BODY_SIZE`] (10 MiB) in `new()`.
pub struct ParseBodyLayer<T> {
    _marker: PhantomData<fn() -> T>,
    max_body_size: usize,
}

impl<T> ParseBodyLayer<T> {
    /// Construct a parser layer for body type `T` with the default
    /// [`DEFAULT_MAX_BODY_SIZE`] (10 MiB) body size limit.
    pub fn new() -> Self {
        Self {
            _marker: PhantomData,
            max_body_size: DEFAULT_MAX_BODY_SIZE,
        }
    }

    /// Set the maximum body size in bytes. Bodies exceeding this limit
    /// will short-circuit with `413 Payload Too Large`.
    pub fn max_body_size(mut self, limit: usize) -> Self {
        self.max_body_size = limit;
        self
    }
}

impl<T> Default for ParseBodyLayer<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Clone for ParseBodyLayer<T> {
    fn clone(&self) -> Self {
        Self {
            _marker: PhantomData,
            max_body_size: self.max_body_size,
        }
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
            max_body_size: self.max_body_size,
        }
    }
}

/// Inner-service wrapper produced by [`ParseBodyLayer::layer`]. Buffers the
/// request body and inserts a parsed `T` into request extensions on success.
///
/// On parse failure (JOLTR-RS-060): the layer short-circuits with a
/// `400 Bad Request` carrying `"Invalid JSON: <serde error>"` as a `text/plain`
/// body, and flips
/// [`RequestExt::mark_finished`](crate::RequestExt::mark_finished) so any
/// composed observers see the same finished-flag contract Router relies on.
/// The inner service is NOT invoked when the parse fails.
pub struct ParseBodyService<S, T> {
    inner: S,
    _marker: PhantomData<fn() -> T>,
    max_body_size: usize,
}

impl<S: Clone, T> Clone for ParseBodyService<S, T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _marker: PhantomData,
            max_body_size: self.max_body_size,
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
        let cloned = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, cloned);

        if let Some(ext) = req.extensions().get::<Arc<RequestExt>>() {
            if ext.is_finished() {
                return Box::pin(async move { inner.call(req).await });
            }
        }

        let max_body_size = self.max_body_size;

        Box::pin(async move {
            let (mut parts, body) = req.into_parts();

            let request_ext: Arc<RequestExt> = match parts.extensions.get::<Arc<RequestExt>>() {
                Some(existing) => Arc::clone(existing),
                None => {
                    let fresh = Arc::new(RequestExt::new());
                    parts.extensions.insert(Arc::clone(&fresh));
                    fresh
                }
            };

            let content_length = body.size_hint().lower() as usize;
            if content_length > max_body_size {
                request_ext.mark_finished();
                let resp = bad_request_for_oversized_body(max_body_size, content_length);
                let _req = AxumRequest::from_parts(parts, Body::empty());
                return Ok(resp);
            }

            let bytes = match buffer_body(body, max_body_size).await {
                Some(b) => b,
                None => {
                    let got = content_length;
                    request_ext.mark_finished();
                    let resp = bad_request_for_oversized_body(max_body_size, got);
                    let _req = AxumRequest::from_parts(parts, Body::empty());
                    return Ok(resp);
                }
            };

            let mut req = AxumRequest::from_parts(parts, Body::from(bytes.clone()));

            match serde_json::from_slice::<T>(&bytes) {
                Ok(parsed) => {
                    req.extensions_mut().insert(parsed);
                    inner.call(req).await
                }
                Err(err) => {
                    request_ext.mark_finished();
                    Ok(bad_request_for_parse_error(&err))
                }
            }
        })
    }
}

/// Drain an axum [`Body`] into [`Bytes`] under the given `max_size` cap,
/// returning `None` if the body exceeds the limit or an I/O error occurs.
/// When the body fits within `max_size`, returns `Some(bytes)`.
async fn buffer_body(body: Body, max_size: usize) -> Option<Bytes> {
    axum::body::to_bytes(body, max_size).await.ok()
}

/// Build the `400 Bad Request` response surfaced when `serde_json::from_slice`
/// rejects the buffered body bytes (JOLTR-RS-060).
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

/// Build the `413 Payload Too Large` response surfaced when the buffered body
/// exceeds the configured `max_body_size` (JOLTR-RS-062).
fn bad_request_for_oversized_body(limit: usize, _got: usize) -> Response {
    let body = format!("Body exceeds maximum allowed size: {limit} bytes");
    Response::builder()
        .status(StatusCode::PAYLOAD_TOO_LARGE)
        .header(
            CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )
        .body(Body::from(body))
        .expect("413 response builder always succeeds with static headers + owned body")
}

/// `tower::Layer` that buffers the request body and decodes it as a UTF-8
/// [`String`] (JOLTR-RS-061). On success, the decoded string is inserted into
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
#[derive(Clone, Debug)]
pub struct ParseBodyStringLayer {
    max_body_size: usize,
}

impl ParseBodyStringLayer {
    /// Construct a string-body parser layer with the default
    /// [`DEFAULT_MAX_BODY_SIZE`] (10 MiB) body size limit.
    pub fn new() -> Self {
        Self {
            max_body_size: DEFAULT_MAX_BODY_SIZE,
        }
    }

    /// Set the maximum body size in bytes. Bodies exceeding this limit
    /// will short-circuit with `413 Payload Too Large`.
    pub fn max_body_size(mut self, limit: usize) -> Self {
        self.max_body_size = limit;
        self
    }
}

impl Default for ParseBodyStringLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> Layer<S> for ParseBodyStringLayer {
    type Service = ParseBodyStringService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        ParseBodyStringService {
            inner,
            max_body_size: self.max_body_size,
        }
    }
}

/// Inner-service wrapper produced by [`ParseBodyStringLayer::layer`]. Buffers
/// the request body and inserts a decoded [`String`] into request extensions
/// on success; short-circuits with a `400 Bad Request` on UTF-8 decode
/// failure, or `413 Payload Too Large` on oversized body (JOLTR-RS-062).
/// See [`ParseBodyStringLayer`] for the architectural contract.
#[derive(Clone, Debug)]
pub struct ParseBodyStringService<S> {
    inner: S,
    max_body_size: usize,
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
        let cloned = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, cloned);

        if let Some(ext) = req.extensions().get::<Arc<RequestExt>>() {
            if ext.is_finished() {
                return Box::pin(async move { inner.call(req).await });
            }
        }

        let max_body_size = self.max_body_size;

        Box::pin(async move {
            let (mut parts, body) = req.into_parts();

            let request_ext: Arc<RequestExt> = match parts.extensions.get::<Arc<RequestExt>>() {
                Some(existing) => Arc::clone(existing),
                None => {
                    let fresh = Arc::new(RequestExt::new());
                    parts.extensions.insert(Arc::clone(&fresh));
                    fresh
                }
            };

            let content_length = body.size_hint().lower() as usize;
            if content_length > max_body_size {
                request_ext.mark_finished();
                let resp = bad_request_for_oversized_body(max_body_size, content_length);
                let _req = AxumRequest::from_parts(parts, Body::empty());
                return Ok(resp);
            }

            let bytes = match buffer_body(body, max_body_size).await {
                Some(b) => b,
                None => {
                    let got = content_length;
                    request_ext.mark_finished();
                    let resp = bad_request_for_oversized_body(max_body_size, got);
                    let _req = AxumRequest::from_parts(parts, Body::empty());
                    return Ok(resp);
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
/// rejects the buffered body bytes (JOLTR-RS-061). The body is `text/plain`,
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
