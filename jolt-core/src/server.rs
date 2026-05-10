//! Top-level Jolt HTTP server configuration.
//!
//! [`JoltServer`] is a plain configuration record carrying port, thread, and
//! optional CORS/TLS settings, an [`EndpointRegistry`] for routes registered
//! via [`JoltServer::endpoint`], plus a [`JoltServer::start`] entry point that
//! binds an axum [`Router`] on `0.0.0.0:port` with graceful shutdown driven by
//! `tokio::signal` (SIGINT plus SIGTERM on unix).
//!
//! [`CorsConfig`] (JOLT-RS-055) carries the four CORS knobs the upcoming
//! [`tower::Layer`](::tower::Layer) impls in JOLT-RS-056..058 read at request
//! time: `allow_origins`, `allow_methods`, `allow_headers`, and `max_age`. The
//! [`Default`] impl produces an empty/restrictive config (no origins, no
//! methods, no headers, `max_age = 0`) — opening up CORS is an explicit
//! caller decision, never the default. [`TlsConfig`] remains a stub marker
//! whose fields are filled in when the TLS phase lands.

use std::net::{Ipv4Addr, SocketAddr};
use std::num::NonZeroUsize;

use axum::Router;

use crate::endpoint::Endpoint;
use crate::endpoint_registry::EndpointRegistry;
use crate::method::Method;
use crate::registered_endpoint::RegisteredEndpoint;

/// Per-server CORS configuration consumed by the CORS [`tower::Layer`] landing
/// in JOLT-RS-056..058.
///
/// Fields mirror the CORS preflight response headers verbatim:
/// - `allow_origins` → `Access-Control-Allow-Origin` candidates. A single `"*"`
///   entry is the conventional wildcard; multiple entries imply per-request
///   origin matching against the request's `Origin` header.
/// - `allow_methods` → `Access-Control-Allow-Methods`. Held as Jolt's
///   [`Method`] enum so the CORS layer can serialize via [`Method::as_str`]
///   without re-parsing strings.
/// - `allow_headers` → `Access-Control-Allow-Headers`.
/// - `max_age` → `Access-Control-Max-Age` (seconds the browser may cache the
///   preflight). `u32` covers RFC 6454's effective range without sign concerns.
///
/// [`Default`] returns an empty/restrictive config: no origins, no methods,
/// no headers, `max_age = 0`. Callers who want permissive behavior must set
/// the fields explicitly — defaults intentionally do NOT enable CORS for any
/// origin, mirroring how a server with no CORS layer at all would respond.
#[derive(Debug, Clone, Default)]
pub struct CorsConfig {
    pub allow_origins: Vec<String>,
    pub allow_methods: Vec<Method>,
    pub allow_headers: Vec<String>,
    pub max_age: u32,
}

#[derive(Debug)]
pub struct TlsConfig;

pub struct JoltServer {
    pub port: u16,
    pub threads: usize,
    pub cors_config: Option<CorsConfig>,
    pub tls_config: Option<TlsConfig>,
    pub registry: EndpointRegistry,
}

impl JoltServer {
    /// Construct a server with sensible defaults: port `8080` and a thread
    /// count matching the host's available parallelism. Falls back to `1` on
    /// the rare platforms where parallelism cannot be determined.
    pub fn new() -> Self {
        let threads = std::thread::available_parallelism()
            .map(NonZeroUsize::get)
            .unwrap_or(1);
        Self {
            port: 8080,
            threads,
            cors_config: None,
            tls_config: None,
            registry: EndpointRegistry::new(),
        }
    }

    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    pub fn threads(mut self, threads: usize) -> Self {
        self.threads = threads;
        self
    }

    pub fn cors(mut self, cors: CorsConfig) -> Self {
        self.cors_config = Some(cors);
        self
    }

    /// Register an endpoint with the server's [`EndpointRegistry`]. The
    /// `Send + Sync + 'static` bounds are inherited from the registry's
    /// trait-object slot — `Endpoint` itself does not require them, but the
    /// registry stores `Box<dyn Endpoint + Send + Sync>`, so the generic must
    /// be at least as strong.
    pub fn endpoint<E: Endpoint + Send + Sync + 'static>(mut self, endpoint: E) -> Self {
        self.registry.register(endpoint);
        self
    }

    /// Collect every [`RegisteredEndpoint`] discovered at compile time via
    /// `#[endpoint(..)]` (across all linked crates) and register it into
    /// [`Self::registry`]. Registration uses the [`Endpoint`] impl on
    /// `&'static RegisteredEndpoint` (see `registered_endpoint.rs`), so no
    /// allocation or boxing is needed for the inventory entries themselves —
    /// the registry only `Arc`s the static reference.
    ///
    /// Idempotent in spirit but NOT enforced: calling twice would push every
    /// entry twice. The intended usage is one call from [`Self::start`] just
    /// before serving; expose it as `pub` so test harnesses (and advanced
    /// users who want to inspect the registry pre-serve) can drive the
    /// collection without binding a port.
    pub fn collect_inventory_endpoints(mut self) -> Self {
        for entry in inventory::iter::<RegisteredEndpoint> {
            self.registry.register(entry);
        }
        self
    }

    /// Build the axum [`Router`] this server will serve: collects every
    /// `inventory::iter::<RegisteredEndpoint>()` entry into the registry,
    /// sorts longest-path-first (so `/api/hello` matches before `/api`), and
    /// returns the registry's compiled router. Used by [`Self::start`] and
    /// exposed publicly so tests can exercise inventory-collected routes
    /// through `tower::ServiceExt::oneshot` without binding a TCP port.
    pub fn into_router(self) -> Router {
        let mut server = self.collect_inventory_endpoints();
        server.registry.sort();
        server.registry.build_router()
    }

    /// Bind the configured router on `0.0.0.0:port` and serve it until a
    /// shutdown signal arrives (SIGINT, plus SIGTERM on unix). Bind failures
    /// (port-in-use, EACCES on low ports) and signal-handler installation
    /// failures both surface as `io::Result::Err`.
    ///
    /// JOLT-RS-044: before binding, every `inventory::iter::<RegisteredEndpoint>()`
    /// entry from any linked crate is registered into [`Self::registry`] and
    /// merged into the user-supplied `extra_router` via [`Router::merge`]. The
    /// merge order is `extra_router.merge(inventory_router)`; axum panics on
    /// route conflicts at the `merge` call, which is the right failure mode
    /// (a duplicate route is a wiring bug, not a runtime condition).
    ///
    /// `self.threads` is currently advisory: the server runs on the caller's
    /// existing tokio runtime, so worker count is whatever that runtime was
    /// built with. A future runtime-build PRD item can wire `threads` through
    /// without changing this signature.
    pub async fn start(self, extra_router: Router) -> std::io::Result<()> {
        let port = self.port;
        let inventory_router = self.into_router();
        let merged = extra_router.merge(inventory_router);
        let addr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, port));
        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, merged)
            .with_graceful_shutdown(shutdown_signal())
            .await
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}
