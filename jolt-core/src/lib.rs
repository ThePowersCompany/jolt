#![doc = "jolt-core: HTTP, WebSocket, SSE, tasks, and pub/sub primitives for the Jolt framework."]

pub mod cookie;
pub mod cors;
pub mod endpoint;
pub mod endpoint_registry;
pub mod method;
pub mod parse_body;
pub mod parse_query;
pub mod registered_endpoint;
pub mod request;
pub mod request_ext;
pub mod response;
pub mod router;
pub mod server;
pub mod status;

pub use cookie::Cookie;
pub use cors::{CorsLayer, CorsService};
pub use endpoint::{Endpoint, EndpointFuture};
pub use endpoint_registry::EndpointRegistry;
pub use method::{Method, ParseMethodError};
pub use parse_body::{ParseBodyLayer, ParseBodyService, ParseBodyStringLayer, ParseBodyStringService};
pub use parse_query::{ParseQueryLayer, ParseQueryService, QueryParams};
pub use registered_endpoint::RegisteredEndpoint;
pub use request::Request;
pub use request_ext::RequestExt;
pub use response::{JsonBody, Response};
pub use router::Router;
pub use server::{CorsConfig, JoltServer, TlsConfig};
pub use status::StatusCode;

// Re-export `inventory` so the `#[endpoint]` macro can emit
// `::jolt_core::inventory::submit!` without forcing every user crate to add
// `inventory` to its own Cargo.toml. Re-export `jolt_macros::endpoint` for the
// same reason — user crates `use jolt_core::endpoint;` instead of pulling in
// jolt-macros directly.
//
// `tower` is re-exported for the `#[derive(AutoMiddleware)]` codegen
// (JOLT-RS-051): the derive emits `::jolt_core::tower::Layer` /
// `::jolt_core::tower::Service` impls so the user's middleware struct slots
// into a tower stack without the user crate having to depend on tower itself.
pub use inventory;
pub use jolt_macros::{endpoint, AutoMiddleware};
pub use tower;

#[cfg(test)]
mod tests;
