//! Top-level Jolt HTTP server configuration.
//!
//! [`JoltServer`] is a plain configuration record at this stage. Builder
//! methods, endpoint registration, and `start` land in subsequent PRD items
//! (JOLT-RS-024..026). [`CorsConfig`] and [`TlsConfig`] are stub markers whose
//! fields are filled in when the corresponding middleware/TLS phases land —
//! keeping them as nameable types now lets the builder surface compile against
//! a stable shape.

use std::num::NonZeroUsize;

#[derive(Debug)]
pub struct CorsConfig;

#[derive(Debug)]
pub struct TlsConfig;

#[derive(Debug)]
pub struct JoltServer {
    pub port: u16,
    pub threads: usize,
    pub cors_config: Option<CorsConfig>,
    pub tls_config: Option<TlsConfig>,
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
        }
    }
}
