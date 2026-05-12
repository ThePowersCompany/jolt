//! Static file-serving `tower::Layer` (PRD items 6 and 7).
//!
//! [`FileServeLayer`] wraps an inner [`tower::Service`] (typically JoltR's
//! [`Router`](crate::Router)) and intercepts requests whose path falls under a
//! configured route prefix, dispatching them to a [`tower_http::services::ServeDir`]
//! rooted at a caller-supplied directory. Requests outside the prefix are
//! delegated to the inner service unchanged.
//!
//! Contract:
//! - **Prefix matching**: a request path matches when it equals the prefix OR
//!   starts with the prefix followed by `/`. The prefix is normalized at
//!   construction so a caller-supplied `"static"` / `"/static"` / `"/static/"`
//!   all canonicalize to `"/static"`. An empty prefix means "serve all
//!   requests from the configured root."
//! - **URI rewriting**: on a match, the request URI's path is rewritten to the
//!   suffix after the prefix (preserving any query string) before being handed
//!   to `ServeDir`, so a `GET /static/foo.txt` request hits `<root>/foo.txt`
//!   rather than `<root>/static/foo.txt`.
//! - **404 for missing files**: delegated to `ServeDir`, which returns a 404
//!   response (with empty body) when the resolved path does not resolve to a
//!   regular file.
//! - **Cache validators**: successful file responses include `Cache-Control`,
//!   `ETag`, and `Last-Modified`. Matching `If-None-Match` requests return
//!   `304 Not Modified` with the same cache validators and no body.
//! - **Range / gzip**: intentionally out of scope until PRD item 8.
//! - **Path traversal**: blocked by `ServeDir`'s built-in path validation —
//!   any URI containing `..` components canonicalizes to a path outside the
//!   configured root, which `ServeDir` refuses to serve.

use std::convert::Infallible;
use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::UNIX_EPOCH;

use axum::body::Body;
use axum::extract::Request as AxumRequest;
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::response::Response;
use chrono::{DateTime, Utc};
use tower::{Layer, Service, ServiceExt};
use tower_http::services::ServeDir;

/// `tower::Layer` that mounts a static file root at a route prefix.
///
/// Cloning is cheap: the layer owns a normalized prefix and the configured
/// root path. Each [`Layer::layer`] call constructs a fresh [`ServeDir`] from
/// that root so produced services don't share mutable state with one another
/// or with the originating layer.
#[derive(Clone, Debug)]
pub struct FileServeLayer {
    prefix: String,
    root: PathBuf,
}

impl FileServeLayer {
    /// Build a layer that serves files under `root` at the given route
    /// `prefix`. The prefix is normalized: leading `/` is added if missing,
    /// trailing `/` is stripped. An empty prefix is permitted and means
    /// "every request is forwarded to `ServeDir`."
    pub fn new(prefix: impl Into<String>, root: impl Into<PathBuf>) -> Self {
        Self {
            prefix: normalize_prefix(prefix.into()),
            root: root.into(),
        }
    }

    /// Borrow the normalized prefix. Exposed for tests + introspection.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }
}

impl<S> Layer<S> for FileServeLayer {
    type Service = FileServeService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        FileServeService {
            inner,
            prefix: self.prefix.clone(),
            root: self.root.clone(),
            serve_dir: ServeDir::new(&self.root),
        }
    }
}

/// Inner-service wrapper produced by [`FileServeLayer::layer`]. Forwards
/// requests under the configured prefix to `ServeDir`; delegates everything
/// else to the wrapped `inner` service.
#[derive(Clone, Debug)]
pub struct FileServeService<S> {
    inner: S,
    prefix: String,
    root: PathBuf,
    serve_dir: ServeDir,
}

impl<S> Service<AxumRequest> for FileServeService<S>
where
    S: Service<AxumRequest, Response = Response, Error = Infallible> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = Response;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response, Infallible>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // Forward to the inner service. `ServeDir` (no fallback) is always
        // ready, so its readiness is asserted at call time via `oneshot`.
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: AxumRequest) -> Self::Future {
        // Mirror the CorsService delegation discipline: replace the live inner
        // with a clone we don't drive, so the caller's next poll_ready/call
        // cycle operates on a fresh slot rather than the one we just consumed.
        let cloned = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, cloned);

        match match_prefix(&self.prefix, req.uri().path()) {
            Some(stripped_path) => {
                let cache_headers = file_cache_headers(&self.root, &stripped_path);
                let if_none_match = req.headers().get(header::IF_NONE_MATCH).cloned();
                let method = req.method().clone();
                *req.uri_mut() = rewrite_uri(req.uri(), &stripped_path);
                let serve_dir = self.serve_dir.clone();
                Box::pin(async move {
                    // `oneshot` polls readiness on the cloned service before
                    // dispatching — the canonical tower pattern when calling
                    // a service inside an async block.
                    let response = serve_dir
                        .oneshot(req)
                        .await
                        .unwrap_or_else(|infallible| match infallible {});

                    let response = if response.status() == StatusCode::OK {
                        if let Some(cache_headers) = cache_headers {
                            if accepts_not_modified(&method, if_none_match.as_ref(), &cache_headers)
                            {
                                return Ok(not_modified_response(&cache_headers));
                            }

                            let mut response = response;
                            insert_cache_headers(response.headers_mut(), &cache_headers);
                            response
                        } else {
                            response
                        }
                    } else {
                        response
                    };

                    // `ServeDir` owns the filesystem body type; adapt it into
                    // axum's standard body after header injection.
                    Ok(response.map(Body::new))
                })
            }
            None => Box::pin(async move { inner.call(req).await }),
        }
    }
}

/// Normalize a caller-supplied prefix to the form the runtime matcher
/// expects: leading `/` present (unless the prefix is empty), no trailing
/// `/`. An empty input stays empty so callers can opt into "serve all
/// requests from the root" without a special API.
fn normalize_prefix(prefix: String) -> String {
    if prefix.is_empty() {
        return prefix;
    }
    let mut p = if prefix.starts_with('/') {
        prefix
    } else {
        format!("/{prefix}")
    };
    while p.len() > 1 && p.ends_with('/') {
        p.pop();
    }
    p
}

/// If `path` falls under `prefix`, return the suffix (always leading `/`)
/// that should be substituted into the URI handed to `ServeDir`. Otherwise
/// return `None` so the request delegates to the inner service.
///
/// Empty `prefix` matches every path and returns it unchanged.
fn match_prefix(prefix: &str, path: &str) -> Option<String> {
    if prefix.is_empty() {
        return Some(path.to_string());
    }
    if path == prefix {
        return Some("/".to_string());
    }
    let rest = path.strip_prefix(prefix)?;
    if rest.starts_with('/') {
        Some(rest.to_string())
    } else {
        // e.g. prefix=`/static`, path=`/staticky` — NOT a prefix hit; without
        // this guard the literal `starts_with` would steal sibling routes.
        None
    }
}

/// Replace the path component of `uri` with `new_path`, preserving query.
/// Falls back to `Uri::from_static("/")` only if the rewritten path+query
/// fails to parse — unreachable in practice because the inputs come from a
/// path we just validated.
fn rewrite_uri(uri: &Uri, new_path: &str) -> Uri {
    let new_pq = match uri.query() {
        Some(q) => format!("{new_path}?{q}"),
        None => new_path.to_string(),
    };
    let mut parts = uri.clone().into_parts();
    parts.path_and_query = new_pq.parse().ok();
    Uri::from_parts(parts).unwrap_or_else(|_| Uri::from_static("/"))
}

#[derive(Clone, Debug)]
struct FileCacheHeaders {
    etag: HeaderValue,
    last_modified: HeaderValue,
}

fn file_cache_headers(root: &Path, stripped_path: &str) -> Option<FileCacheHeaders> {
    let path = resolve_static_file_path(root, stripped_path)?;
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }

    let modified = metadata.modified().ok()?;
    let modified_since_epoch = modified.duration_since(UNIX_EPOCH).unwrap_or_default();
    let modified_nanos = u128::from(modified_since_epoch.as_secs()) * 1_000_000_000
        + u128::from(modified_since_epoch.subsec_nanos());
    let etag =
        HeaderValue::from_str(&format!("W/\"{:x}-{modified_nanos:x}\"", metadata.len())).ok()?;

    let modified_at: DateTime<Utc> = modified.into();
    let last_modified =
        HeaderValue::from_str(&modified_at.format("%a, %d %b %Y %H:%M:%S GMT").to_string()).ok()?;

    Some(FileCacheHeaders {
        etag,
        last_modified,
    })
}

fn resolve_static_file_path(root: &Path, stripped_path: &str) -> Option<PathBuf> {
    let mut path = root.to_path_buf();
    for component in Path::new(stripped_path.trim_start_matches('/')).components() {
        match component {
            Component::Normal(segment) => path.push(segment),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(path)
}

fn accepts_not_modified(
    method: &Method,
    if_none_match: Option<&HeaderValue>,
    cache_headers: &FileCacheHeaders,
) -> bool {
    (method == Method::GET || method == Method::HEAD)
        && if_none_match
            .map(|value| if_none_match_matches(value, &cache_headers.etag))
            .unwrap_or(false)
}

fn if_none_match_matches(value: &HeaderValue, etag: &HeaderValue) -> bool {
    let Ok(value) = value.to_str() else {
        return false;
    };
    let Ok(etag) = etag.to_str() else {
        return false;
    };
    let weak_etag = strip_weak_etag(etag);

    value.split(',').any(|candidate| {
        let candidate = candidate.trim();
        candidate == "*" || strip_weak_etag(candidate) == weak_etag
    })
}

fn strip_weak_etag(etag: &str) -> &str {
    etag.strip_prefix("W/").unwrap_or(etag)
}

fn insert_cache_headers(headers: &mut HeaderMap, cache_headers: &FileCacheHeaders) {
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=0"),
    );
    headers.insert(header::ETAG, cache_headers.etag.clone());
    headers.insert(header::LAST_MODIFIED, cache_headers.last_modified.clone());
}

fn not_modified_response(cache_headers: &FileCacheHeaders) -> Response {
    let mut response = Response::builder()
        .status(StatusCode::NOT_MODIFIED)
        .body(Body::empty())
        .expect("static 304 response builds");
    insert_cache_headers(response.headers_mut(), cache_headers);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_prefix_adds_leading_slash() {
        assert_eq!(normalize_prefix("static".into()), "/static");
    }

    #[test]
    fn normalize_prefix_strips_trailing_slashes() {
        assert_eq!(normalize_prefix("/static/".into()), "/static");
        assert_eq!(normalize_prefix("/static//".into()), "/static");
    }

    #[test]
    fn normalize_prefix_preserves_empty() {
        assert_eq!(normalize_prefix(String::new()), "");
    }

    #[test]
    fn normalize_prefix_preserves_root_slash() {
        // `/` is the boundary of the "strip trailing slash" rule — don't
        // collapse the root prefix to the empty-string "match everything"
        // shape, which is semantically distinct.
        assert_eq!(normalize_prefix("/".into()), "/");
    }

    #[test]
    fn match_prefix_strips_prefix_and_keeps_suffix() {
        assert_eq!(
            match_prefix("/static", "/static/foo.txt"),
            Some("/foo.txt".to_string())
        );
    }

    #[test]
    fn match_prefix_handles_exact_prefix_path() {
        assert_eq!(match_prefix("/static", "/static"), Some("/".to_string()));
    }

    #[test]
    fn match_prefix_rejects_sibling_prefix() {
        // `/staticky` is NOT served by a `/static` mount; the suffix must
        // start with `/` or be empty.
        assert_eq!(match_prefix("/static", "/staticky/foo"), None);
    }

    #[test]
    fn match_prefix_rejects_unrelated_path() {
        assert_eq!(match_prefix("/static", "/api/health"), None);
    }

    #[test]
    fn match_prefix_empty_matches_anything() {
        assert_eq!(match_prefix("", "/anything"), Some("/anything".to_string()));
    }

    #[test]
    fn rewrite_uri_preserves_query_string() {
        let uri: Uri = "/static/foo.txt?v=1".parse().expect("parses");
        let rewritten = rewrite_uri(&uri, "/foo.txt");
        assert_eq!(rewritten.path(), "/foo.txt");
        assert_eq!(rewritten.query(), Some("v=1"));
    }

    #[test]
    fn rewrite_uri_without_query_keeps_no_query() {
        let uri: Uri = "/static/foo.txt".parse().expect("parses");
        let rewritten = rewrite_uri(&uri, "/foo.txt");
        assert_eq!(rewritten.path(), "/foo.txt");
        assert_eq!(rewritten.query(), None);
    }
}
