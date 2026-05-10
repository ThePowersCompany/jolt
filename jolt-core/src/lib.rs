#![doc = "jolt-core: HTTP, WebSocket, SSE, tasks, and pub/sub primitives for the Jolt framework."]

pub mod auth_bearer;
pub mod auth_jwt;
pub mod auth_websocket;
pub mod auth_ws_jwt;
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

pub use auth_bearer::{AuthBearerLayer, AuthBearerService, BearerToken};
pub use auth_jwt::{AuthJwtLayer, AuthJwtService};
// JOLT-RS-076 surface: the WS-JWT auth precheck layer composing the 075
// extractor with the 072 decoder. On success the layer stashes a
// [`WsJwtToken`] AND a [`JwtClaims`] into request extensions for 077's WS
// handler to read.
pub use auth_ws_jwt::{AuthWsJwtLayer, AuthWsJwtService};
// JOLT-RS-075 surface: the WS subprotocol token extractor + its rejection
// enum + the typed WsJwtToken handle that JOLT-RS-076's tower::Layer will
// stash into request extensions for JOLT-RS-077 to read.
pub use auth_websocket::{
    extract_jwt_token as extract_ws_jwt_token, WsJwtToken, WsTokenRejectReason,
    JOLT_JWT_SUBPROTOCOL,
};
// Re-export jolt-utils JWT types at the jolt-core surface so user crates that
// consume `AuthJwtLayer` (JOLT-RS-072) don't need a direct `jolt-utils` dep
// just to build a `JwtConfig` or read a `JwtClaims` from extensions.
pub use jolt_utils::jwt::{JwtClaims, JwtConfig, JwtDecodeError};
pub use cookie::Cookie;
pub use cors::{CorsLayer, CorsService};
pub use endpoint::{Endpoint, EndpointFuture};
pub use endpoint_registry::EndpointRegistry;
pub use method::{Method, ParseMethodError};
pub use parse_body::{ParseBodyLayer, ParseBodyService, ParseBodyStringLayer, ParseBodyStringService};
pub use parse_query::{
    bad_request_for_query_error, extract as extract_query, extract_bool as extract_query_bool,
    extract_enum as extract_query_enum, extract_string as extract_query_string,
    extract_vec as extract_query_vec, ParseQueryLayer, ParseQueryService, QueryExtractError,
    QueryParams,
};
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
