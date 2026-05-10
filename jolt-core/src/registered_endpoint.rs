//! Static registration record for endpoints discovered at compile time.
//!
//! Each `#[get]`/`#[post]`/etc. method inside a `#[endpoint("/path")]`-decorated
//! impl emits one [`RegisteredEndpoint`] via `inventory::submit!`.
//! [`crate::JoltServer::start`] (JOLT-RS-044) iterates
//! `inventory::iter::<RegisteredEndpoint>()` and feeds each entry into the
//! server's [`crate::EndpointRegistry`] before binding the listener.
//!
//! `path` is `&'static str` (not `String`) so the entire record can live in a
//! `static` slot, which is how `inventory::submit!` collects entries — a
//! `String` would force runtime allocation in the static-init phase, which the
//! crate explicitly forbids.
//!
//! `handler` is a bare `fn` pointer (added in JOLT-RS-044), not a closure or a
//! trait object: `inventory::submit!` requires the value to be constructible
//! in `const` context, and only `fn` pointers satisfy that. The macro emits
//! `<UserType>::__jolt_handler_<user_fn>` (JOLT-RS-043's wrapper) into this
//! field, which has the right `fn(Request) -> EndpointFuture` shape.
//!
//! [`Endpoint`] is implemented for `&'static RegisteredEndpoint`, so the
//! registry can directly hold inventory references without a wrapping newtype:
//! `inventory::iter::<RegisteredEndpoint>()` already yields `&'static
//! RegisteredEndpoint`s, and the registry's `Send + Sync + 'static` bound is
//! satisfied (the underlying record is Send + Sync via its Copy fields).
//!
//! Stored fields are `pub` rather than going through accessors because every
//! known consumer (server start, dump-routes diagnostics) reads all three
//! fields together — accessors would just be one-line passthroughs.

use crate::endpoint::{Endpoint, EndpointFuture};
use crate::method::Method;
use crate::request::Request;

#[derive(Debug)]
pub struct RegisteredEndpoint {
    pub path: &'static str,
    pub method: Method,
    pub handler: fn(Request) -> EndpointFuture,
}

inventory::collect!(RegisteredEndpoint);

/// Bridges an inventory-collected `&'static RegisteredEndpoint` into the
/// runtime [`Endpoint`] trait the [`crate::EndpointRegistry`] stores. The
/// registry takes `Box<dyn Endpoint + Send + Sync>` trait objects; rather
/// than introducing a newtype wrapper (`FnEndpoint`), we implement `Endpoint`
/// directly on the static reference. `fn` pointers are `Send + Sync`, and
/// `&'static T` is `Send + Sync + 'static` whenever `T: Send + Sync`, so this
/// satisfies the registry's bounds without an `Arc`/`Box` allocation.
impl Endpoint for &'static RegisteredEndpoint {
    fn path(&self) -> &str {
        self.path
    }

    fn method(&self) -> Method {
        self.method
    }

    fn handler(&self, req: Request) -> EndpointFuture {
        (self.handler)(req)
    }
}
