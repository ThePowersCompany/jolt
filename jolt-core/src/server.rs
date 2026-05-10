//! Top-level Jolt HTTP server configuration.
//!
//! [`JoltServer`] is a plain configuration record carrying port, thread, and
//! optional CORS/TLS settings, an [`EndpointRegistry`] for routes registered
//! via [`JoltServer::endpoint`], plus a [`JoltServer::start`] entry point that
//! binds an axum [`Router`] on `0.0.0.0:port` with graceful shutdown driven by
//! `tokio::signal` (SIGINT plus SIGTERM on unix). [`CorsConfig`] and
//! [`TlsConfig`] remain stub markers whose fields are filled in when the
//! corresponding middleware/TLS phases land — keeping them as nameable types
//! now lets the builder surface compile against a stable shape.

use std::net::{Ipv4Addr, SocketAddr};
use std::num::NonZeroUsize;

use axum::Router;

use crate::endpoint::Endpoint;
use crate::endpoint_registry::EndpointRegistry;

#[derive(Debug)]
pub struct CorsConfig;

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

    /// Bind the configured router on `0.0.0.0:port` and serve it until a
    /// shutdown signal arrives (SIGINT, plus SIGTERM on unix). Bind failures
    /// (port-in-use, EACCES on low ports) and signal-handler installation
    /// failures both surface as `io::Result::Err`.
    ///
    /// `self.threads` is currently advisory: the server runs on the caller's
    /// existing tokio runtime, so worker count is whatever that runtime was
    /// built with. A future runtime-build PRD item can wire `threads` through
    /// without changing this signature.
    pub async fn start(self, router: Router) -> std::io::Result<()> {
        let addr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, self.port));
        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, router)
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
