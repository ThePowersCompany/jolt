//! Query string parsing `tower::Layer` (JOLT-RS-063).
//!
//! [`ParseQueryLayer`] reads the inbound request's URI query string, splits it
//! into key/value pairs, and stashes the resulting
//! [`QueryParams`] (a `HashMap<String, String>` newtype) into request
//! extensions so a downstream service (or the AutoMiddleware-derived struct
//! consuming the request) can pull it back out with
//! `req.extensions().get::<QueryParams>()`.
//!
//! Architectural decisions pinned here for JOLT-RS-064..067 to build on:
//!
//! 1. **Layer carries no type parameter; output is always
//!    [`QueryParams`].** JOLT-RS-063 mandates "key-value pairs" only — typed
//!    extraction (int/float, bool, enum, `Vec<T>`) lands in 064–066 as
//!    *consumers* of this map. The map is the foundation, not a typed shape.
//!    A future `ParseQueryLayer<T>` over a deserializable struct (mirroring
//!    [`ParseBodyLayer<T>`](crate::ParseBodyLayer)) is a sibling layer, not a
//!    parameterization of this one — it would target a different decoding
//!    surface (`serde_urlencoded::from_str`) and have a different failure mode.
//!
//! 2. **`QueryParams` is a newtype over `HashMap<String, String>`, not the
//!    raw `HashMap` itself.** Inserting a bare `HashMap<String, String>` into
//!    request extensions would collide with any other layer or handler that
//!    happens to stash one for a different purpose (request extensions are
//!    keyed by `TypeId`). The newtype gives this layer a unique extension key
//!    while still being one `Deref` from the underlying map for ergonomics.
//!
//! 3. **Empty / missing query string inserts an empty [`QueryParams`].** The
//!    extension is ALWAYS present after the layer runs; downstream consumers
//!    can call `.get::<QueryParams>().unwrap()` (or expect-with-message) without
//!    a `?query=` upstream. Making the extension conditional on a non-empty
//!    query would force every consumer to handle two shapes (present-empty vs.
//!    absent) for the same logical state.
//!
//! 4. **Parsing is infallible.** Malformed pairs (no `=`, repeated `&`, etc.)
//!    are silently dropped — same shape as
//!    [`endpoint_registry::parse_query`](crate::endpoint_registry) which has
//!    been the framework's de-facto query parser since JOLT-RS-034. Rejecting
//!    a `?foo&bar=1` query as 400 would be more strict than the existing Jolt
//!    [`Request::query`](crate::Request::query) contract; a future
//!    typed-extractor layer (064+) can choose to surface 400 on per-value type
//!    errors without changing the foundational map's permissive shape.
//!
//! 5. **No body buffering, no `Arc<RequestExt>` preserve-or-inject.** Unlike
//!    [`ParseBodyLayer`](crate::ParseBodyLayer) and
//!    [`CorsLayer`](crate::CorsLayer), this layer doesn't fail and doesn't
//!    short-circuit, so it has no reason to flip a finished latch. The
//!    request flows through unchanged except for the inserted extension.
//!    064+ typed extractors that DO surface 400s will need the preserve-or-
//!    inject dance; this foundational layer does not.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::extract::Request as AxumRequest;
use axum::response::Response;
use tower::{Layer, Service};

/// Newtype wrapper over `HashMap<String, String>` used as the request-extension
/// key for the parsed query map (JOLT-RS-063). The newtype shields downstream
/// consumers from collisions with any other `HashMap<String, String>` that
/// might be stashed in extensions for unrelated reasons. `Deref` exposes the
/// underlying map's read API directly; no separate forwarding methods needed.
///
/// `Default` is derived so [`ParseQueryService::call`] can produce an empty
/// instance without manual construction.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueryParams(pub HashMap<String, String>);

impl std::ops::Deref for QueryParams {
    type Target = HashMap<String, String>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for QueryParams {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<HashMap<String, String>> for QueryParams {
    fn from(map: HashMap<String, String>) -> Self {
        Self(map)
    }
}

/// `tower::Layer` that parses the request URI's query string into a
/// [`QueryParams`] map and stashes it in request extensions. See module docs
/// for the architectural contract (extension key, infallibility, always-insert
/// shape).
///
/// Carries no runtime state; cloning produces a functionally identical layer.
#[derive(Default, Clone, Debug)]
pub struct ParseQueryLayer;

impl ParseQueryLayer {
    /// Construct a query parser layer. The layer carries no runtime state, so
    /// a fresh layer is functionally identical to any other.
    pub fn new() -> Self {
        Self
    }
}

impl<S> Layer<S> for ParseQueryLayer {
    type Service = ParseQueryService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        ParseQueryService { inner }
    }
}

/// Inner-service wrapper produced by [`ParseQueryLayer::layer`]. Inserts the
/// parsed [`QueryParams`] into request extensions before delegating to the
/// inner service. See [`ParseQueryLayer`] for the architectural contract.
#[derive(Clone, Debug)]
pub struct ParseQueryService<S> {
    inner: S,
}

impl<S> Service<AxumRequest> for ParseQueryService<S>
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

    fn call(&mut self, mut req: AxumRequest) -> Self::Future {
        // Standard tower delegation pattern: poll_ready was driven on the
        // current `self.inner`, so `call` must use that same instance. Replace
        // it with a clone we DON'T call; the caller's next poll_ready readies
        // that slot. Same idiom as `ParseBodyService::call` (JOLT-RS-059) and
        // `CorsService::call` (JOLT-RS-056).
        let cloned = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, cloned);

        let params = QueryParams(parse_query(req.uri().query()));
        req.extensions_mut().insert(params);

        Box::pin(async move { inner.call(req).await })
    }
}

/// Split a query string of the form `k=v&k=v` into a key→value map. Mirrors
/// [`endpoint_registry::parse_query`](crate::endpoint_registry) verbatim so
/// the layer's behavior is observably identical to the existing `Request`
/// snapshot path's interpretation of the same URL.
///
/// `None` (no `?` in the URI) returns an empty map. Malformed pairs (any chunk
/// without an `=`) are silently dropped — see module docs decision 4 for the
/// rationale.
///
/// Percent-decoding is NOT performed; the same caveat applies to the existing
/// `endpoint_registry::parse_query`. A future polish item can hoist a shared
/// decoder used by both call sites.
fn parse_query(query: Option<&str>) -> HashMap<String, String> {
    let Some(q) = query else {
        return HashMap::new();
    };
    q.split('&')
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            Some((k.to_string(), v.to_string()))
        })
        .collect()
}
