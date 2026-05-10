//! Static registration record for endpoints discovered at compile time.
//!
//! Each `#[get]`/`#[post]`/etc. method inside a `#[endpoint("/path")]`-decorated
//! impl emits one [`RegisteredEndpoint`] via `inventory::submit!`. JOLT-RS-044
//! will iterate `inventory::iter::<RegisteredEndpoint>()` from
//! `JoltServer::start` and feed each entry into an [`crate::EndpointRegistry`].
//!
//! For JOLT-RS-042 the record carries only the route key — `path` and
//! `method`. The handler function pointer is added in JOLT-RS-043 once the
//! handler-wrapper codegen lands; until then the integration test verifies
//! the registration plumbing end-to-end on metadata only.
//!
//! `path` is `&'static str` (not `String`) so the entire record can live in a
//! `static` slot, which is how `inventory::submit!` collects entries — a
//! `String` would force runtime allocation in the static-init phase, which the
//! crate explicitly forbids.
//!
//! Stored fields are `pub` rather than going through accessors because every
//! known consumer (server start, dump-routes diagnostics) reads all three
//! fields together — accessors would just be one-line passthroughs.

use crate::method::Method;

#[derive(Debug)]
pub struct RegisteredEndpoint {
    pub path: &'static str,
    pub method: Method,
}

inventory::collect!(RegisteredEndpoint);
