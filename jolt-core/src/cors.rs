//! CORS preflight short-circuit `tower::Layer` (JOLT-RS-056).
//!
//! [`CorsLayer`] wraps an inner [`tower::Service`] (typically Jolt's
//! [`Router`](crate::Router)) and intercepts `OPTIONS` requests, returning a
//! `204 No Content` response carrying the four preflight headers built from
//! [`CorsConfig`]:
//!
//! - `Access-Control-Allow-Origin`
//! - `Access-Control-Allow-Methods`
//! - `Access-Control-Allow-Headers`
//! - `Access-Control-Max-Age`
//!
//! Non-`OPTIONS` requests are delegated to the inner service; on the way back,
//! [`Access-Control-Allow-Origin`] (first entry of `allow_origins`, mirroring
//! the preflight rule from JOLT-RS-056) and [`Access-Control-Expose-Headers`]
//! (joined `allow_origins` whitelist) are injected onto the inner's response
//! when the matching config field is non-empty (JOLT-RS-057). Existing values
//! on the response are not overwritten — if the inner service has already set
//! either header, the layer leaves it alone.
//!
//! [`Access-Control-Allow-Origin`]: axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN
//! [`Access-Control-Expose-Headers`]: axum::http::header::ACCESS_CONTROL_EXPOSE_HEADERS
//!
//! On a preflight short-circuit, the layer also flips
//! [`RequestExt::mark_finished`](crate::RequestExt::mark_finished) on the
//! request's existing [`Arc<RequestExt>`](crate::RequestExt) (or freshly
//! injects one if no upstream layer has) so any composed observers see the
//! same finished-flag contract Router relies on for its own short-circuit
//! path. Because this layer returns the preflight response directly from its
//! `call()`, the inner service is never invoked on `OPTIONS` and the
//! `finished` flag is purely an observability signal — it does NOT round-trip
//! through Router's stash/take dance.
//!
//! Empty/restrictive defaults match the [`CorsConfig::default`] contract from
//! JOLT-RS-055: a config whose `allow_origins`/`allow_methods`/`allow_headers`
//! are empty and `max_age == 0` produces a bare 204 with no CORS headers,
//! equivalent to having no CORS layer at all (no origin is granted access).
//! Permissive behavior requires the caller to set fields explicitly.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::body::Body;
use axum::extract::Request as AxumRequest;
use axum::http::header::{
    ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
    ACCESS_CONTROL_EXPOSE_HEADERS, ACCESS_CONTROL_MAX_AGE,
};
use axum::http::{HeaderValue, Method as HttpMethod, StatusCode};
use axum::response::Response;
use tower::{Layer, Service};

use crate::method::Method;
use crate::request_ext::RequestExt;
use crate::server::CorsConfig;

/// `tower::Layer` carrying a [`CorsConfig`] used to build preflight responses.
///
/// `Clone` is required by the standard `tower::ServiceBuilder` composition
/// path and follows tower 0.5's Layer convention. The internal `CorsConfig`
/// is itself `Clone`, so each [`Layer::layer`] call hands a fresh owned copy
/// to the produced [`CorsService`] — services own their config independent of
/// the originating layer.
#[derive(Clone, Debug)]
pub struct CorsLayer {
    config: CorsConfig,
}

impl CorsLayer {
    /// Build a layer from an explicit [`CorsConfig`]. The caller's config is
    /// consumed (not borrowed) so the layer doesn't need to hold a reference
    /// for the lifetime of the service stack.
    pub fn new(config: CorsConfig) -> Self {
        Self { config }
    }
}

impl<S> Layer<S> for CorsLayer {
    type Service = CorsService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        CorsService {
            inner,
            config: self.config.clone(),
        }
    }
}

/// Inner-service wrapper produced by [`CorsLayer::layer`]. On `OPTIONS`,
/// short-circuits with a 204 preflight response; on every other method,
/// delegates to the inner service unchanged.
#[derive(Clone, Debug)]
pub struct CorsService<S> {
    inner: S,
    config: CorsConfig,
}

impl<S> Service<AxumRequest> for CorsService<S>
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
        // Mirror Router's preserve-or-inject contract from JOLT-RS-035 so the
        // finished flag we set is observable to whoever holds the same Arc
        // (callers, tests, or downstream layers). Inserting a fresh ext when
        // none is present means a CorsLayer composed OUTSIDE a Router still
        // satisfies the spec's "Set RequestExt::mark_finished()" requirement.
        let request_ext: Arc<RequestExt> = match req.extensions().get::<Arc<RequestExt>>() {
            Some(existing) => Arc::clone(existing),
            None => {
                let fresh = Arc::new(RequestExt::new());
                req.extensions_mut().insert(Arc::clone(&fresh));
                fresh
            }
        };

        if req.method() == HttpMethod::OPTIONS {
            let response = build_preflight_response(&self.config);
            request_ext.mark_finished();
            return Box::pin(async move { Ok(response) });
        }

        // Standard tower delegation pattern: poll_ready was driven on the
        // *current* `self.inner`, so `call` must use that same instance —
        // not a fresh clone. Replace the inner with a clone we DON'T call;
        // the caller's next poll_ready will ready THAT cloned slot before
        // their next call.
        let cloned = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, cloned);
        let config = self.config.clone();
        Box::pin(async move {
            let mut response = inner.call(req).await?;
            inject_response_cors_headers(&config, &mut response);
            Ok(response)
        })
    }
}

/// Inject `Access-Control-Allow-Origin` and `Access-Control-Expose-Headers`
/// onto a non-preflight response per JOLT-RS-057. Mutates the response in
/// place; both headers are skipped when the corresponding `CorsConfig` field
/// is empty (matching the OPTIONS branch's empty-default contract from
/// JOLT-RS-056). If the inner service has already set either header, the
/// existing value is preserved — the layer is additive, not authoritative.
fn inject_response_cors_headers(config: &CorsConfig, response: &mut Response) {
    let headers = response.headers_mut();

    // `Access-Control-Allow-Origin` mirrors the preflight first-entry rule
    // (JOLT-RS-056). Per-request `Origin`-header matching against multiple
    // configured origins is JOLT-RS-058's territory and will replace this
    // simplification with a shared helper used by both the OPTIONS and
    // non-OPTIONS branches. For now: empty config → no header; one or more
    // origins → emit the first, which handles the common single-origin and
    // `vec!["*"]` wildcard shapes.
    if !headers.contains_key(ACCESS_CONTROL_ALLOW_ORIGIN) {
        if let Some(origin) = config.allow_origins.first() {
            if let Ok(value) = HeaderValue::from_str(origin) {
                headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, value);
            }
        }
    }

    // `Access-Control-Expose-Headers` is a comma-joined whitelist of response
    // header names a browser-side script may read across the CORS boundary.
    // Empty config → no header (preserves the JOLT-RS-055 restrictive-default
    // contract; absence means the browser falls back to the safe-header set).
    if !headers.contains_key(ACCESS_CONTROL_EXPOSE_HEADERS) && !config.expose_headers.is_empty() {
        let joined = config.expose_headers.join(", ");
        if let Ok(value) = HeaderValue::from_str(&joined) {
            headers.insert(ACCESS_CONTROL_EXPOSE_HEADERS, value);
        }
    }
}

/// Render a 204 preflight response from a [`CorsConfig`]. Each header is
/// emitted only when the corresponding config field is non-empty / non-zero,
/// so the [`CorsConfig::default`] empty/restrictive shape produces a bare 204
/// without granting any CORS access — matching the "default never opens up
/// CORS" contract pinned by JOLT-RS-055.
fn build_preflight_response(config: &CorsConfig) -> Response {
    let mut builder = Response::builder().status(StatusCode::NO_CONTENT);

    // `Access-Control-Allow-Origin` carries a single value per the spec; if
    // multiple origins are configured, request-Origin matching (JOLT-RS-058)
    // will pick the right one. For 056 we surface the first configured entry
    // — `vec!["*"]` becomes a literal `*`, a single specific origin echoes
    // back as itself. Empty config => no header.
    if let Some(origin) = config.allow_origins.first() {
        if let Ok(value) = HeaderValue::from_str(origin) {
            builder = builder.header(ACCESS_CONTROL_ALLOW_ORIGIN, value);
        }
    }

    if !config.allow_methods.is_empty() {
        let joined = config
            .allow_methods
            .iter()
            .map(Method::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        // Method::as_str returns ASCII uppercase verbs only; HeaderValue
        // rejection is unreachable in practice but the result is checked
        // anyway to avoid a panic if a future Method variant introduces
        // non-visible-ASCII bytes.
        if let Ok(value) = HeaderValue::from_str(&joined) {
            builder = builder.header(ACCESS_CONTROL_ALLOW_METHODS, value);
        }
    }

    if !config.allow_headers.is_empty() {
        let joined = config.allow_headers.join(", ");
        if let Ok(value) = HeaderValue::from_str(&joined) {
            builder = builder.header(ACCESS_CONTROL_ALLOW_HEADERS, value);
        }
    }

    // `max_age == 0` is the restrictive default — emitting `Max-Age: 0`
    // would tell the browser to never cache the preflight, which is an
    // opinionated stance to take by default. Skip the header when the
    // caller hasn't opted in.
    if config.max_age > 0 {
        builder = builder.header(ACCESS_CONTROL_MAX_AGE, HeaderValue::from(config.max_age));
    }

    builder
        .body(Body::empty())
        .expect("204 preflight response builder always produces a valid response")
}
