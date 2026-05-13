//! DEBUG-level request/response body logging (JOLTR-RS-070).
//!
//! [`BodyLogLayer`] buffers request and response bodies, logs them at
//! [`tracing::debug`] level (truncated to [`MAX_BODY_LOG_BYTES`] printable
//! characters), and passes the buffered body through unchanged. Sensitive
//! path prefixes (e.g. `/auth`) suppress body logging entirely.
//!
//! The layer sits between [`TraceLayer`] and the router so buffering happens
//! after auth/CORS/parse-body middleware have had their say but before the
//! trace span is created.
//!
//! Body capture is best-effort: bodies are buffered up to
//! [`DEFAULT_MAX_BODY_SIZE`] (10 MiB, mirroring [`ParseBodyLayer`]'s cap).
//! Exceeding that limit skips logging and discards the body — which is safe
//! only because downstream [`ParseBodyLayer`] would reject it with 413 anyway.
//! For endpoints without a ParseBodyLayer, debug body logging of ultra-large
//! payloads is silently skipped and the body is lost (a known limitation of
//! the "buffer then log" approach).
//!
//! [`TraceLayer`]: tower_http::trace::TraceLayer
//! [`ParseBodyLayer`]: crate::ParseBodyLayer

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::body::Body;
use axum::extract::Request as AxumRequest;
use axum::response::Response;
use tower::{Layer, Service};

use crate::parse_body::DEFAULT_MAX_BODY_SIZE;
use crate::request_ext::RequestExt;

const MAX_BODY_LOG_BYTES: usize = 1024;

const SENSITIVE_PATH_PATTERNS: &[&str] = &["/auth"];

// ---------------------------------------------------------------------------
// Layer
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct BodyLogLayer;

impl<S> Layer<S> for BodyLogLayer {
    type Service = BodyLogService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        BodyLogService { inner }
    }
}

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct BodyLogService<S> {
    inner: S,
}

impl<S> Service<AxumRequest> for BodyLogService<S>
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

        let path = req.uri().path().to_string();
        let is_sensitive = SENSITIVE_PATH_PATTERNS.iter().any(|p| path.contains(p));
        let debug_enabled = tracing::enabled!(tracing::Level::DEBUG);
        let should_log = !is_sensitive && debug_enabled;

        Box::pin(async move {
            let (parts, body) = req.into_parts();

            let req_bytes = axum::body::to_bytes(body, DEFAULT_MAX_BODY_SIZE).await.ok();

            if should_log {
                if let Some(ref bytes) = req_bytes {
                    log_body("REQ", &path, bytes);
                }
            }

            let body = match req_bytes {
                Some(bytes) => Body::from(bytes),
                None => Body::empty(),
            };
            let req = AxumRequest::from_parts(parts, body);

            let resp = inner.call(req).await?;

            if !should_log {
                return Ok(resp);
            }

            let (resp_parts, resp_body) = resp.into_parts();
            let resp_bytes = axum::body::to_bytes(resp_body, DEFAULT_MAX_BODY_SIZE)
                .await
                .ok();

            if let Some(ref bytes) = resp_bytes {
                log_body("RESP", &path, bytes);
            }

            let body = match resp_bytes {
                Some(bytes) => Body::from(bytes),
                None => Body::empty(),
            };
            Ok(Response::from_parts(resp_parts, body))
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn log_body(direction: &str, path: &str, bytes: &[u8]) {
    let (preview, suffix) = if bytes.len() > MAX_BODY_LOG_BYTES {
        (
            String::from_utf8_lossy(&bytes[..MAX_BODY_LOG_BYTES]).into_owned(),
            "...<truncated>",
        )
    } else {
        (String::from_utf8_lossy(bytes).into_owned(), "")
    };

    tracing::debug!(
        direction = direction,
        path = %path,
        body_bytes = bytes.len(),
        body = %format_args!("{preview}{suffix}"),
    );
}
