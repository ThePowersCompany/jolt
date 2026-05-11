#![doc = "jolt-core: HTTP, WebSocket, SSE, tasks, and pub/sub primitives for the Jolt framework."]

pub mod auth_error;
pub mod auth_bearer;
pub mod auth_jwt;
pub mod auth_websocket;
pub mod auth_ws_jwt;
pub mod body_log;
pub mod cookie;
pub mod cors;
pub mod endpoint;
pub mod endpoint_registry;
pub mod method;
pub mod optional;
pub mod parse_body;
pub mod parse_query;
pub mod registered_endpoint;
pub mod request;
pub mod request_ext;
pub mod response;
pub mod router;
pub mod server;
pub mod status;
pub mod to_sql;
pub mod websocket;

pub use auth_bearer::{AuthBearerLayer, AuthBearerService, BearerToken};
pub use auth_error::AuthError;
pub use auth_jwt::{AuthJwtLayer, AuthJwtService};
pub use body_log::BodyLogLayer;
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
pub use optional::Optional;
pub use to_sql::ToSql;
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
// JOLT-RS-118 surface: the WebSocket lifecycle trait (five async callbacks
// with no-op defaults).
// JOLT-RS-119 surface: the real `WebSocketSender` (mpsc-backed cheap-clone
// handle with send/send_text/send_json/close methods) and its `WsSendError`
// companion.
// JOLT-RS-120 surface: the `WsMessage` variants (Text/Binary/Ping/Pong/Close),
// the Jolt-owned `CloseFrame`, and the `From<axum::Message> for WsMessage`
// mapping (plus the inverse) that 124's read/write loops will exercise.
// JOLT-RS-122 surface: the hidden `__WsMacroWitness` struct that the `ws!`
// macro's expansion constructs. Not part of the stable API surface; carried
// only so 122's integration test can verify the macro parsed + expanded.
// Re-exported at the crate root so user crates `use jolt_core::WebSocketHandler;`
// without needing to know the internal module layout.
pub use websocket::{CloseFrame, WebSocketHandler, WebSocketSender, WsMessage, WsSendError, __WsMacroWitness};

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
// JOLT-RS-122 surface: the `ws!` function-like proc-macro for declaring an
// axum WebSocket route with a JWT-subprotocol auth gate. Re-exported here so
// user crates `use jolt_core::ws;` instead of pulling in `jolt-macros`
// directly.
pub use jolt_macros::{endpoint, ws, AutoMiddleware, PatchQuery};
// Re-export futures-util so the `ws!` macro's generated code (JOLT-RS-124)
// can reference `::jolt_core::futures_util::SinkExt` / `StreamExt` without
// forcing every user crate to add `futures-util` to its own Cargo.toml.
pub use futures_util;
pub use tower;

#[cfg(test)]
mod tests;
