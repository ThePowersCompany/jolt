//! PRD item 6 — `FileServeLayer` static file-serving contract.
//!
//! Exercises end-to-end through `tower::ServiceExt::oneshot`:
//! - a path under the configured prefix serves the matching file with 200 +
//!   its bytes,
//! - a path under the prefix that doesn't resolve to a regular file returns
//!   404 (delegated to `ServeDir`),
//! - a path outside the prefix delegates to the wrapped inner service
//!   unchanged (no URI rewrite, no file lookup),
//! - a path-traversal attempt (`..` segments) does not escape the configured
//!   root and is rejected by `ServeDir` as 404,
//! - a query string survives the URI rewrite step.
//!
//! Each test stages a fresh temp directory under `target/file-serve-<nanos>-N`
//! so the tests are isolated, deterministic, and (on assertion failure) the
//! artifacts can be inspected manually.

use std::convert::Infallible;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::{to_bytes, Body};
use axum::http::{header, Request as AxumRequest, StatusCode};
use axum::response::Response;
use joltr_core::FileServeLayer;
use tower::{Layer, Service, ServiceExt};

/// Inner service used to verify the "delegates outside the prefix" path: it
/// responds with a unique 418 + sentinel body so a test assertion can
/// distinguish "inner was called" from "ServeDir returned a fallback."
#[derive(Clone)]
struct SentinelInner;

const SENTINEL_BODY: &[u8] = b"sentinel-inner";

impl Service<AxumRequest<Body>> for SentinelInner {
    type Response = Response;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response, Infallible>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _req: AxumRequest<Body>) -> Self::Future {
        Box::pin(async move {
            Ok(Response::builder()
                .status(StatusCode::IM_A_TEAPOT)
                .body(Body::from(SENTINEL_BODY))
                .expect("static teapot response builds"))
        })
    }
}

/// Allocate a fresh test directory under `target/`. Per-test suffix counter
/// keeps multiple tests in the same run from colliding; nanos timestamp
/// keeps repeated runs from colliding with stale dirs.
fn fresh_test_dir() -> PathBuf {
    static SUFFIX: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time is after epoch")
        .as_nanos();
    let suffix = SUFFIX.fetch_add(1, Ordering::Relaxed);
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join(format!("file-serve-{nanos}-{suffix}"));
    std::fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn write_file(dir: &Path, name: &str, contents: &[u8]) {
    std::fs::write(dir.join(name), contents).expect("write fixture file");
}

fn get(path: &str) -> AxumRequest<Body> {
    AxumRequest::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .expect("test request builds")
}

fn get_with_if_none_match(path: &str, etag: &str) -> AxumRequest<Body> {
    AxumRequest::builder()
        .method("GET")
        .uri(path)
        .header(header::IF_NONE_MATCH, etag)
        .body(Body::empty())
        .expect("test request builds")
}

#[tokio::test]
async fn serves_file_when_path_matches_prefix() {
    let root = fresh_test_dir();
    write_file(&root, "hello.txt", b"hello world");

    let svc = FileServeLayer::new("/static", &root).layer(SentinelInner);
    let response = svc
        .oneshot(get("/static/hello.txt"))
        .await
        .expect("service is infallible");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024)
        .await
        .expect("body collects");
    assert_eq!(&body[..], b"hello world");
}

#[tokio::test]
async fn served_file_includes_cache_headers() {
    let root = fresh_test_dir();
    write_file(&root, "hello.txt", b"hello world");

    let svc = FileServeLayer::new("/static", &root).layer(SentinelInner);
    let response = svc
        .oneshot(get("/static/hello.txt"))
        .await
        .expect("service is infallible");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("public, max-age=0")
    );

    let etag = response
        .headers()
        .get(header::ETAG)
        .and_then(|value| value.to_str().ok())
        .expect("served file includes ETag");
    assert!(etag.starts_with("W/\""));
    assert!(etag.ends_with('"'));

    let last_modified = response
        .headers()
        .get(header::LAST_MODIFIED)
        .and_then(|value| value.to_str().ok())
        .expect("served file includes Last-Modified");
    assert!(last_modified.ends_with(" GMT"));
}

#[tokio::test]
async fn matching_if_none_match_returns_304_without_body() {
    let root = fresh_test_dir();
    write_file(&root, "hello.txt", b"hello world");

    let svc = FileServeLayer::new("/static", &root).layer(SentinelInner);
    let first = svc
        .oneshot(get("/static/hello.txt"))
        .await
        .expect("service is infallible");
    let etag = first
        .headers()
        .get(header::ETAG)
        .and_then(|value| value.to_str().ok())
        .expect("first response includes ETag")
        .to_string();

    let svc = FileServeLayer::new("/static", &root).layer(SentinelInner);
    let response = svc
        .oneshot(get_with_if_none_match("/static/hello.txt", &etag))
        .await
        .expect("service is infallible");

    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(
        response
            .headers()
            .get(header::ETAG)
            .and_then(|value| value.to_str().ok()),
        Some(etag.as_str())
    );
    assert_eq!(
        response
            .headers()
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("public, max-age=0")
    );
    assert!(response.headers().contains_key(header::LAST_MODIFIED));

    let body = to_bytes(response.into_body(), 1024)
        .await
        .expect("body collects");
    assert!(body.is_empty());
}

#[tokio::test]
async fn returns_404_for_missing_file_under_prefix() {
    let root = fresh_test_dir();
    // No files staged — every request under the prefix should 404.

    let svc = FileServeLayer::new("/static", &root).layer(SentinelInner);
    let response = svc
        .oneshot(get("/static/does-not-exist.txt"))
        .await
        .expect("service is infallible");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delegates_to_inner_when_path_does_not_match_prefix() {
    let root = fresh_test_dir();
    write_file(&root, "hello.txt", b"unused");

    let svc = FileServeLayer::new("/static", &root).layer(SentinelInner);
    let response = svc
        .oneshot(get("/api/health"))
        .await
        .expect("service is infallible");

    // The sentinel inner returns 418 with a recognizable body — proves the
    // request actually went through `inner.call`, not through ServeDir's
    // own miss-path branch (which would surface 404 with an empty body).
    assert_eq!(response.status(), StatusCode::IM_A_TEAPOT);
    let body = to_bytes(response.into_body(), 1024)
        .await
        .expect("body collects");
    assert_eq!(&body[..], SENTINEL_BODY);
}

#[tokio::test]
async fn sibling_prefix_is_not_a_match() {
    // `/staticky` shares a literal `/static` prefix but is NOT under the
    // mount — must delegate to inner, not be treated as a ServeDir lookup
    // for `<root>/y/foo`.
    let root = fresh_test_dir();

    let svc = FileServeLayer::new("/static", &root).layer(SentinelInner);
    let response = svc
        .oneshot(get("/staticky/foo"))
        .await
        .expect("service is infallible");

    assert_eq!(response.status(), StatusCode::IM_A_TEAPOT);
}

#[tokio::test]
async fn path_traversal_does_not_escape_root() {
    let root = fresh_test_dir();
    // Plant a file OUTSIDE the configured root that a naive impl might
    // accidentally serve via `..`. The test root is `<workspace>/target/
    // file-serve-<…>`, so `../<sibling>` would try to read a sibling test
    // directory — which we don't create, but the path validation must reject
    // the traversal before the filesystem is consulted regardless.
    write_file(&root, "inside.txt", b"safe");

    let svc = FileServeLayer::new("/static", &root).layer(SentinelInner);
    let response = svc
        .oneshot(get("/static/../inside.txt"))
        .await
        .expect("service is infallible");

    // `ServeDir` canonicalizes the path and rejects any traversal
    // component → 404. The inside.txt file IS readable when requested
    // through the legitimate `/static/inside.txt` path (see the
    // companion sanity assertion below).
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // Sanity check: the legitimate route still works after the traversal
    // attempt — guards against a regression that bans the prefix
    // wholesale on any `..` in any request.
    let svc = FileServeLayer::new("/static", &root).layer(SentinelInner);
    let ok = svc
        .oneshot(get("/static/inside.txt"))
        .await
        .expect("service is infallible");
    assert_eq!(ok.status(), StatusCode::OK);
}

#[tokio::test]
async fn query_string_survives_rewrite() {
    let root = fresh_test_dir();
    write_file(&root, "hello.txt", b"hello");

    let svc = FileServeLayer::new("/static", &root).layer(SentinelInner);
    // ServeDir doesn't read the query, but the rewrite path should not
    // mangle the URI shape — request still resolves to `hello.txt`.
    let response = svc
        .oneshot(get("/static/hello.txt?v=42"))
        .await
        .expect("service is infallible");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024)
        .await
        .expect("body collects");
    assert_eq!(&body[..], b"hello");
}

#[tokio::test]
async fn prefix_without_leading_slash_is_normalized() {
    let root = fresh_test_dir();
    write_file(&root, "hello.txt", b"hello");

    // Caller wrote `static` (no leading `/`). The layer should normalize to
    // `/static` so the standard request shape still matches.
    let svc = FileServeLayer::new("static", &root).layer(SentinelInner);
    let response = svc
        .oneshot(get("/static/hello.txt"))
        .await
        .expect("service is infallible");

    assert_eq!(response.status(), StatusCode::OK);
}
