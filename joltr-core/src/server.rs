//! Top-level JoltR HTTP server configuration.
//!
//! [`JoltRServer`] is a plain configuration record carrying port, thread, and
//! optional CORS/TLS settings, an [`EndpointRegistry`] for routes registered
//! via [`JoltRServer::endpoint`], plus [`JoltRServer::start`] and
//! [`JoltRServer::start_blocking`] entry points that bind an axum [`Router`] on
//! `0.0.0.0:port` with graceful shutdown driven by `tokio::signal` (SIGINT plus
//! SIGTERM on unix).
//!
//! [`CorsConfig`] (JOLTR-RS-055) carries the four CORS knobs the upcoming
//! [`tower::Layer`](::tower::Layer) impls in JOLTR-RS-056..058 read at request
//! time: `allow_origins`, `allow_methods`, `allow_headers`, and `max_age`. The
//! [`Default`] impl produces an empty/restrictive config (no origins, no
//! methods, no headers, `max_age = 0`) — opening up CORS is an explicit
//! caller decision, never the default. [`TlsConfig`] carries certificate/key
//! paths used by the rustls startup path when TLS is configured.

use std::future::Future;
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::Router;
use axum_server::tls_rustls::RustlsConfig;
use tower_http::trace::TraceLayer;

use crate::body_log::BodyLogLayer;
use crate::endpoint::Endpoint;
use crate::endpoint_registry::EndpointRegistry;
use crate::method::Method;
use crate::registered_endpoint::RegisteredEndpoint;

/// Per-server CORS configuration consumed by the CORS [`tower::Layer`] landing
/// in JOLTR-RS-056..058.
///
/// Fields mirror the CORS preflight response headers verbatim:
/// - `allow_origins` → `Access-Control-Allow-Origin` candidates. A single `"*"`
///   entry is the conventional wildcard; multiple entries imply per-request
///   origin matching against the request's `Origin` header.
/// - `allow_methods` → `Access-Control-Allow-Methods`. Held as JoltR's
///   [`Method`] enum so the CORS layer can serialize via [`Method::as_str`]
///   without re-parsing strings.
/// - `allow_headers` → `Access-Control-Allow-Headers`.
/// - `max_age` → `Access-Control-Max-Age` (seconds the browser may cache the
///   preflight). `u32` covers RFC 6454's effective range without sign concerns.
/// - `expose_headers` → `Access-Control-Expose-Headers` (JOLTR-RS-057). Names
///   of response headers a browser-side script may read across the CORS
///   boundary. Empty `Vec` means no header is emitted (the browser falls back
///   to its `Access-Control-Expose-Headers` whitelist of safe headers).
///
/// [`Default`] returns an empty/restrictive config: no origins, no methods,
/// no headers, `max_age = 0`, no exposed headers. Callers who want permissive
/// behavior must set the fields explicitly — defaults intentionally do NOT
/// enable CORS for any origin, mirroring how a server with no CORS layer at
/// all would respond.
#[derive(Debug, Clone, Default)]
pub struct CorsConfig {
    pub allow_origins: Vec<String>,
    pub allow_methods: Vec<Method>,
    pub allow_headers: Vec<String>,
    pub max_age: u32,
    pub expose_headers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsConfig {
    pub cert_chain_path: PathBuf,
    pub private_key_path: PathBuf,
}

pub struct JoltRServer {
    pub port: u16,
    pub threads: usize,
    pub cors_config: Option<CorsConfig>,
    pub tls_config: Option<TlsConfig>,
    pub registry: EndpointRegistry,
}

impl JoltRServer {
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

    pub fn tls(mut self, tls: TlsConfig) -> Self {
        self.tls_config = Some(tls);
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

    /// Build the full serving stack: collect inventory endpoints, merge with
    /// the user-supplied `extra_router`, and wrap the result in a customized
    /// [`TraceLayer`] that emits INFO-level log lines in the form
    /// `METHOD /path STATUS Xms` (JOLTR-RS-068 / JOLTR-RS-069).
    pub fn build_serving_router(self, extra_router: Router) -> Router {
        let inventory_router = self.into_router();

        let trace_layer = TraceLayer::new_for_http()
            .make_span_with(|request: &axum::http::Request<axum::body::Body>| {
                tracing::info_span!(
                    "request",
                    method = %request.method(),
                    uri = %request.uri().path(),
                )
            })
            .on_response(
                |response: &axum::http::Response<axum::body::Body>,
                 latency: Duration,
                 _span: &tracing::Span| {
                    let _enter = _span.enter();
                    tracing::info!(
                        status = response.status().as_u16(),
                        latency_ms = latency.as_millis() as u64,
                    );
                },
            );

        let body_log_layer = BodyLogLayer::default();

        extra_router
            .merge(inventory_router)
            .layer(body_log_layer)
            .layer(trace_layer)
    }

    /// Bind the configured router on `0.0.0.0:port` and serve it until a
    /// shutdown signal arrives (SIGINT, plus SIGTERM on unix). Bind failures
    /// (port-in-use, EACCES on low ports) and signal-handler installation
    /// failures both surface as `io::Result::Err`.
    ///
    /// JOLTR-RS-044: before binding, every `inventory::iter::<RegisteredEndpoint>()`
    /// entry from any linked crate is registered into [`Self::registry`] and
    /// merged into the user-supplied `extra_router` via [`Router::merge`]. The
    /// merge order is `extra_router.merge(inventory_router)`; axum panics on
    /// route conflicts at the `merge` call, which is the right failure mode
    /// (a duplicate route is a wiring bug, not a runtime condition).
    ///
    /// JOLTR-RS-068: the merged router is wrapped in
    /// [`TraceLayer::new_for_http`] via [`Self::build_serving_router`] before
    /// being handed to the plain HTTP or TLS serving path, so every served
    /// request emits a tower-http trace span.
    ///
    /// JOLTR-RS-069: a [`tracing_subscriber::fmt`] subscriber with compact
    /// format is installed (if none already exists) so those trace spans
    /// render as human-readable log lines.
    ///
    /// `self.threads` is intentionally ignored here: this async entry point
    /// runs on the caller's existing tokio runtime. Use
    /// [`Self::start_blocking`] when JoltR should own the runtime and apply the
    /// configured worker-thread count.
    pub async fn start(self, extra_router: Router) -> std::io::Result<()> {
        init_tracing_subscriber();

        let port = self.port;
        let tls_config = self.tls_config.clone();
        let serving = self.build_serving_router(extra_router);
        let addr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, port));
        serve_configured(addr, serving, tls_config, shutdown_signal()).await
    }

    /// Build a Tokio multi-thread runtime using [`Self::threads`] and run the
    /// async [`Self::start`] path to completion. This is the entry point for
    /// binaries that do not already own a Tokio runtime.
    pub fn start_blocking(self, extra_router: Router) -> std::io::Result<()> {
        let runtime = self.build_owned_runtime()?;
        runtime.block_on(self.start(extra_router))
    }

    fn build_owned_runtime(&self) -> std::io::Result<tokio::runtime::Runtime> {
        if self.threads == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "JoltRServer::threads must be greater than 0",
            ));
        }

        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(self.threads)
            .enable_all()
            .build()
    }
}

async fn serve_configured<S>(
    addr: SocketAddr,
    serving: Router,
    tls_config: Option<TlsConfig>,
    shutdown: S,
) -> io::Result<()>
where
    S: Future<Output = ()> + Send + 'static,
{
    match tls_config {
        Some(tls) => serve_tls(addr, serving, tls, shutdown).await,
        None => serve_plain(addr, serving, shutdown).await,
    }
}

async fn serve_plain<S>(addr: SocketAddr, serving: Router, shutdown: S) -> io::Result<()>
where
    S: Future<Output = ()> + Send + 'static,
{
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, serving)
        .with_graceful_shutdown(shutdown)
        .await
}

async fn serve_tls<S>(
    addr: SocketAddr,
    serving: Router,
    tls: TlsConfig,
    shutdown: S,
) -> io::Result<()>
where
    S: Future<Output = ()> + Send + 'static,
{
    let rustls_config = rustls_config_from_tls_config(&tls).await?;
    let handle = axum_server::Handle::new();
    let server = axum_server::bind_rustls(addr, rustls_config)
        .handle(handle.clone())
        .serve(serving.into_make_service());
    tokio::pin!(server);

    tokio::select! {
        result = &mut server => result,
        _ = shutdown => {
            handle.graceful_shutdown(None);
            server.await
        }
    }
}

async fn rustls_config_from_tls_config(tls: &TlsConfig) -> io::Result<RustlsConfig> {
    ensure_tls_file_readable(&tls.cert_chain_path, "TLS certificate chain").await?;
    ensure_tls_file_readable(&tls.private_key_path, "TLS private key").await?;
    RustlsConfig::from_pem_file(&tls.cert_chain_path, &tls.private_key_path)
        .await
        .map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid TLS certificate/key configuration: {err}"),
            )
        })
}

async fn ensure_tls_file_readable(path: &Path, label: &str) -> io::Result<()> {
    let metadata = tokio::fs::metadata(path).await.map_err(|err| {
        io::Error::new(
            err.kind(),
            format!("{label} path '{}' is not readable: {err}", path.display()),
        )
    })?;

    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{label} path '{}' must be a file", path.display()),
        ));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        if metadata.permissions().mode() & 0o444 == 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("{label} path '{}' is not readable", path.display()),
            ));
        }
    }

    tokio::fs::File::open(path)
        .await
        .map(|_| ())
        .map_err(|err| {
            io::Error::new(
                err.kind(),
                format!("{label} path '{}' is not readable: {err}", path.display()),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    const TEST_CERT: &str = "-----BEGIN CERTIFICATE-----\n\
MIIDCTCCAfGgAwIBAgIUdZHJ5Kf5Zi7oIlRfw0YHKK7kaywwDQYJKoZIhvcNAQEL\n\
BQAwFDESMBAGA1UEAwwJbG9jYWxob3N0MB4XDTI2MDUxMjE2MjYxNVoXDTI2MDUx\n\
MzE2MjYxNVowFDESMBAGA1UEAwwJbG9jYWxob3N0MIIBIjANBgkqhkiG9w0BAQEF\n\
AAOCAQ8AMIIBCgKCAQEA5xG5LivSFtY8AbPs0vSHxMd6+cjFPXRBgB7zRJ54ClIU\n\
KIIdE67+Jup35Pu3M7idZfymhQEdpWcNvkEpCSXxbathYdCtWWaXQM8BpAtmk92L\n\
8ACtjsjxD+oLTAZmAYkv98dzxaEdiblOg7pEBUO1rDk6J+LLk6heWUzQw2v0+1GQ\n\
2UgOqMtvn/zpaa06bS2XRZuzYA20Ii/iwj2l9q2v5cECxlkcBODAxThZODXlFwPJ\n\
tRkwLcJO7VSl1pdXQsvWxPW2pcLZjiM7vGQSrVoEvZuvBKA1FIuGOa2HFuGUN2Kq\n\
pztMNpHkDf/ubal7nuY71rxUt9VuJKtKCs9igW2sxQIDAQABo1MwUTAdBgNVHQ4E\n\
FgQUu6uHYXpfC8xGBEDxHfThvvVJX4EwHwYDVR0jBBgwFoAUu6uHYXpfC8xGBEDx\n\
HfThvvVJX4EwDwYDVR0TAQH/BAUwAwEB/zANBgkqhkiG9w0BAQsFAAOCAQEASloq\n\
q5WE3QSxocnKA+ECJa4oLFNJ9xtANFx9PWp6+aCjAEWkh1VZplg8vzbb73EpeMQ9\n\
NwwoCLlzg4tJS88IHzPxOFwHGBINDEoXW8pWAOsU+p1FcHvnppcXpHDm+UMAlFyM\n\
AT0RNz1sR3/aoyZe4wXSu1nm/sLcy5+AfLtrYwvuuiLXf5yhGwfU1fBKNvWDsBU+\n\
Isg4rC/b/1lzm5LbXPHyV6wXo/GGif4PjiVax7s/gfy2+LiQm0mfrKJRpzojkE0h\n\
JDUFpdcvPttpdHW7hkXl0kThC/8L+ThzbgM++DEiMPDEuvBmwdGRkHrcCUZ2FYIg\n\
Nq8gbSwEEs86P+G7uw==\n\
-----END CERTIFICATE-----\n";

    const TEST_KEY: &str = "-----BEGIN PRIVATE KEY-----\n\
MIIEvAIBADANBgkqhkiG9w0BAQEFAASCBKYwggSiAgEAAoIBAQDnEbkuK9IW1jwB\n\
s+zS9IfEx3r5yMU9dEGAHvNEnngKUhQogh0Trv4m6nfk+7czuJ1l/KaFAR2lZw2+\n\
QSkJJfFtq2Fh0K1ZZpdAzwGkC2aT3YvwAK2OyPEP6gtMBmYBiS/3x3PFoR2JuU6D\n\
ukQFQ7WsOTon4suTqF5ZTNDDa/T7UZDZSA6oy2+f/OlprTptLZdFm7NgDbQiL+LC\n\
PaX2ra/lwQLGWRwE4MDFOFk4NeUXA8m1GTAtwk7tVKXWl1dCy9bE9balwtmOIzu8\n\
ZBKtWgS9m68EoDUUi4Y5rYcW4ZQ3YqqnO0w2keQN/+5tqXue5jvWvFS31W4kq0oK\n\
z2KBbazFAgMBAAECggEAFOMBPi8v2mibG/xLl/YKn7r4XsPLMHblxNMHYliAuU4a\n\
DF9WiYTz7EidddFbCg3SDHn+A1/Kcj0SfPQe5XjMjgFD2Deos8AWRAFyQIXXS95m\n\
29bz2UhIoguh4cliRxlDgL5CtMaPiPd7JWUc6Ozp6xtxKlHR3RMHFENhS4jiqCTX\n\
UqoGvW8OzDstNrCAZMOwQFCdCazuvIYFRCkvK8ZK9deClDXTriOdw53V8usYqn82\n\
/OJUdjx0jlWXapVnw3iazeoMtV8igU3V4JQihNwSM2it3iQ5lk6MeMVTwbcEo3W4\n\
Ha6KXAqzcRv8igECxfc5YwhBOwxWXxQi8iCPb4nwsQKBgQD+vnvsr2mrmA36H45G\n\
+g1O1WmFcgLnoYkeiIs7M+4PXpJVGzykmUxe96eusHLDrZOvNNOjQ2nKWZ+KEzh3\n\
zjOtuGoHaE/eS5rfCZKH6ZF2LtmqSwgxh83j21LqCI+QR6D+wiRqTCigrPiRLWVU\n\
hQW7TAkb2ulDIvQ4aisT/yNljwKBgQDoNVvjbG0qC/8e6s2TSXbXC2IsyzaDfotg\n\
vSJ06vX1ZQXdvAiXvu07+wsnTMXqB28AkYAXLQdT4kOjntzImb4XIs6jrwW94IIs\n\
tojO//+Q08ahYjYEIbymX6i4wRtKhxOfpeHQWHmIs9oBNJ4tXj3fV6L2V+2pkApM\n\
TQAy2QEmawKBgA0bZ+zpqZo2nKF99igYvY6M1H0VPKkGWrPDesZ1CY6o4waV/CzL\n\
5m4MeAg48+61g5/ELA9+bHcI3XiRPJqNdYM+tdXjvBhH85P+PhfYIK/gATsN2UIT\n\
JrjOlBmOjzRxOwtgmdcOTBjYTYHKwCf1IDJe2ykfvljB96bfb4/71KppAoGAZel9\n\
QawFoS2Qs6YDwX+9m5XQaa5qZ32zeCqqI7w3ZJMI5w8uikLuZaPBemcUeaj3i71Q\n\
BVvq2mL0oqwHZDctmYPaQzZ+g540I6GdG5R5Orvnck7jPB7+Z9qV9zgxXR05fJjW\n\
KZWbCWc958eLIIg7CXmx+QIQOZoRGvm0e5yIB68CgYAn9f5DKN9/4nfKuFfcjq8h\n\
OL4p+t0a8acgv/RkaXlsIf5we5RcNwfjavBRkFxXexI0Cv0To2IXawId3ytC+2DH\n\
du2CqitncSr8ObFGp6kazdYbTCSh5eZYP4q/sf2+k2mrjO0pMj7XA3Ato5NW/zi9\n\
NZP3uANaD/XvAdpmTHmVDg==\n\
-----END PRIVATE KEY-----\n";

    #[test]
    fn build_owned_runtime_uses_configured_thread_count() {
        let runtime = JoltRServer::new()
            .threads(2)
            .build_owned_runtime()
            .expect("runtime should build");

        assert_eq!(runtime.metrics().num_workers(), 2);
    }

    #[test]
    fn build_owned_runtime_rejects_zero_threads() {
        let err = JoltRServer::new()
            .threads(0)
            .build_owned_runtime()
            .expect_err("zero worker threads should be invalid");

        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn tls_config_carries_certificate_and_key_paths() {
        let config = TlsConfig {
            cert_chain_path: PathBuf::from("certs/fullchain.pem"),
            private_key_path: PathBuf::from("certs/privkey.pem"),
        };

        assert_eq!(config.cert_chain_path, PathBuf::from("certs/fullchain.pem"));
        assert_eq!(config.private_key_path, PathBuf::from("certs/privkey.pem"));
    }

    #[test]
    fn tls_builder_wraps_arg_in_some_without_starting_tls() {
        let config = TlsConfig {
            cert_chain_path: PathBuf::from("certs/fullchain.pem"),
            private_key_path: PathBuf::from("certs/privkey.pem"),
        };

        let server = JoltRServer::new().tls(config.clone());

        assert_eq!(server.tls_config, Some(config));
    }

    #[tokio::test]
    async fn configured_tls_path_loads_rustls_and_starts_server() {
        let dir = unique_tls_fixture_dir();
        std::fs::create_dir_all(&dir).expect("create tls fixture dir");
        let cert_path = dir.join("cert.pem");
        let key_path = dir.join("key.pem");
        std::fs::write(&cert_path, TEST_CERT).expect("write cert fixture");
        std::fs::write(&key_path, TEST_KEY).expect("write key fixture");

        let result = serve_configured(
            SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
            Router::new(),
            Some(TlsConfig {
                cert_chain_path: cert_path,
                private_key_path: key_path,
            }),
            async {
                tokio::time::sleep(Duration::from_millis(10)).await;
            },
        )
        .await;

        let _ = std::fs::remove_dir_all(dir);
        result.expect("tls server should start and shut down cleanly");
    }

    fn unique_tls_fixture_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("joltr-core-tls-{}-{nanos}", std::process::id()))
    }

    #[test]
    fn start_blocking_surfaces_bind_errors_without_external_runtime() {
        let probe = std::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
        let port = probe.local_addr().unwrap().port();

        let err = JoltRServer::new()
            .port(port)
            .threads(1)
            .start_blocking(Router::new())
            .expect_err("start_blocking should surface the bind failure");

        assert_eq!(err.kind(), io::ErrorKind::AddrInUse);
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

/// Install a [`tracing_subscriber::fmt`] subscriber with compact, minimal
/// output if no global subscriber is already set. Idempotent: subsequent calls
/// are no-ops.
///
/// Log lines follow the shape `METHOD /path STATUS Xms` (e.g.
/// `GET /api/test 200 4`) — method and URI are span fields emitted by the
/// custom [`TraceLayer::make_span_with`] in [`JoltRServer::build_serving_router`],
/// status and latency are event fields recorded in the tower-http
/// `on_response` callback.
///
/// The subscriber is configured with:
/// - compact format (single-line per event)
/// - no target / no module path (shorter lines)
/// - no thread IDs / thread names (server threads are uninteresting)
/// - `env_filter` defaulting to `info` level unless `RUST_LOG` overrides
fn init_tracing_subscriber() {
    use tracing_subscriber::fmt;
    use tracing_subscriber::EnvFilter;

    let _ = fmt()
        .compact()
        .with_target(false)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .try_init();
}
