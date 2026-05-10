//! Top-level Jolt HTTP server configuration.
//!
//! [`JoltServer`] is a plain configuration record at this stage. Defaults,
//! builder methods, endpoint registration, and `start` land in subsequent PRD
//! items (JOLT-RS-023..026). [`CorsConfig`] and [`TlsConfig`] are stub markers
//! whose fields are filled in when the corresponding middleware/TLS phases
//! land — keeping them as nameable types now lets the builder surface compile
//! against a stable shape.

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
