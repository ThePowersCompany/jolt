//! Storage layer for HTTP endpoints registered with a Jolt server.
//!
//! [`EndpointRegistry`] owns reference-counted [`Endpoint`] trait objects and
//! is the collection that backs [`crate::JoltServer::endpoint`] (JOLT-RS-026).
//! JOLT-RS-029 landed the struct + `register`, JOLT-RS-030 added `sort` +
//! `iter`, and JOLT-RS-031 added [`Self::build_router`] which converts the
//! registered endpoints into an [`axum::Router`].
//!
//! `Send + Sync` are pinned at the trait-object site (`Arc<dyn Endpoint +
//! Send + Sync>`) rather than as supertraits on [`Endpoint`] itself. See
//! `endpoint.rs` for the rationale. Storage uses [`Arc`] (not [`Box`]) so
//! [`Self::build_router`]'s per-route handler closures can clone a reference
//! per request rather than consuming the registry.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::Request as AxumRequest;
use axum::http::HeaderMap;
use axum::routing::{delete, get, head, options, patch, post, put, MethodRouter};
use axum::Router;

use crate::cookie::Cookie;
use crate::endpoint::Endpoint;
use crate::method::Method;
use crate::request::Request;

#[derive(Default)]
pub struct EndpointRegistry {
    endpoints: Vec<Arc<dyn Endpoint + Send + Sync>>,
}

impl EndpointRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Wrap and store an endpoint. Generic over the concrete type so callers
    /// can write `registry.register(MyEndpoint)` without an explicit
    /// `Arc::new`; `Send + Sync + 'static` are required because the
    /// trait-object slot adds those auto-trait bounds.
    pub fn register<E: Endpoint + Send + Sync + 'static>(&mut self, endpoint: E) {
        self.endpoints.push(Arc::new(endpoint));
    }

    pub fn len(&self) -> usize {
        self.endpoints.len()
    }

    pub fn is_empty(&self) -> bool {
        self.endpoints.is_empty()
    }

    /// Sort registered endpoints by path length, longest first, so that
    /// [`Self::build_router`] dispatches `/api/hello` before `/api`.
    /// Uses a stable sort: equal-length paths keep their insertion order.
    pub fn sort(&mut self) {
        self.endpoints.sort_by_key(|e| Reverse(e.path().len()));
    }

    /// Read-only walk over registered endpoints in current order.
    /// Used by tests to verify [`Self::sort`] and as the data source for
    /// [`Self::build_router`].
    pub fn iter(&self) -> impl Iterator<Item = &(dyn Endpoint + Send + Sync)> {
        self.endpoints.iter().map(|a| a.as_ref())
    }

    /// Convert the registered endpoints into an [`axum::Router`]. Iterates the
    /// endpoints in their current order — call [`Self::sort`] first if
    /// longest-path-first matching matters for the routes you registered.
    ///
    /// An empty registry returns [`Router::new`] (404 on all paths). Each
    /// endpoint is wrapped in a closure that builds a Jolt [`Request`] from
    /// the incoming axum request, awaits the endpoint's handler future, and
    /// returns the resulting [`Response`].
    pub fn build_router(&self) -> Router {
        let mut router = Router::new();
        for endpoint in &self.endpoints {
            let path = endpoint.path().to_string();
            let method_router = method_router_for(endpoint.method(), Arc::clone(endpoint));
            router = router.route(&path, method_router);
        }
        router
    }
}

fn method_router_for(method: Method, endpoint: Arc<dyn Endpoint + Send + Sync>) -> MethodRouter {
    let handler = move |req: AxumRequest| {
        let endpoint = Arc::clone(&endpoint);
        async move {
            let jolt_req = build_jolt_request(req).await;
            endpoint.handler(jolt_req).await
        }
    };
    match method {
        Method::Get => get(handler),
        Method::Post => post(handler),
        Method::Put => put(handler),
        Method::Patch => patch(handler),
        Method::Delete => delete(handler),
        Method::Options => options(handler),
        Method::Head => head(handler),
    }
}

/// Convert an inbound [`AxumRequest`] into a Jolt [`Request`] snapshot. Used by
/// [`EndpointRegistry::build_router`]'s per-route closures and by
/// [`crate::router::Router`]'s registry-driven dispatch path (JOLT-RS-034). The
/// extraction is currently minimal — see the body for caveats around URL
/// decoding and the temporary `u32::MAX` body cap.
pub(crate) async fn build_jolt_request(req: AxumRequest) -> Request {
    let (parts, body) = req.into_parts();
    let method = parts
        .method
        .as_str()
        .parse::<Method>()
        .unwrap_or(Method::Get);
    let path = parts.uri.path().to_string();
    let query_params = parse_query(parts.uri.query());
    let cookies = parse_cookies(&parts.headers);
    // Per-request body cap: u32::MAX bytes. Real ceiling will be configurable
    // when middleware/limits land; for now this is a safety valve, not policy.
    let body = axum::body::to_bytes(body, u32::MAX as usize)
        .await
        .map(|b| b.to_vec())
        .unwrap_or_default();
    Request {
        method,
        path,
        headers: parts.headers,
        query_params,
        body,
        cookies,
        finished: false,
    }
}

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

fn parse_cookies(headers: &HeaderMap) -> Vec<Cookie> {
    headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .map(|s| {
            s.split(';')
                .filter_map(|kv| {
                    let (k, v) = kv.split_once('=')?;
                    Some(Cookie {
                        name: k.trim().to_string(),
                        value: v.trim().to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

