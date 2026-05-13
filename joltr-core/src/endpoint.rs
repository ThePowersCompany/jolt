//! Contract every HTTP route registered with a JoltR server implements.
//!
//! Implementors expose a static `path`/`method` pair plus an async `handler`
//! that consumes a JoltR [`Request`] and returns an axum [`Response`]. The
//! trait is consumed by [`crate::JoltRServer::endpoint`] (JOLTR-RS-026) and the
//! `EndpointRegistry` (JOLTR-RS-029).

use std::future::Future;
use std::pin::Pin;

use axum::response::Response;

use crate::method::Method;
use crate::request::Request;

/// Type-erased boxed future returned from [`Endpoint::handler`]. Aliased so
/// implementors and callers can name the return shape once instead of
/// re-typing the full `Pin<Box<dyn Future<...> + Send>>` at every site.
pub type EndpointFuture = Pin<Box<dyn Future<Output = Response> + Send>>;

/// HTTP endpoint registered with a [`crate::JoltRServer`].
///
/// The supertrait list is intentionally empty: `Send + Sync` are NOT required
/// at the trait level because the registry layer (JOLTR-RS-029) attaches them
/// at the trait-object site as `Box<dyn Endpoint + Send + Sync>`. Keeping them
/// off the trait itself preserves the option of holding endpoint values in
/// non-shared contexts (e.g. a single-threaded test harness) without the
/// auto-trait tax.
pub trait Endpoint {
    fn path(&self) -> &str;
    fn method(&self) -> Method;
    fn handler(&self, req: Request) -> EndpointFuture;
}
