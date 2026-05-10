#![doc = "jolt-core: HTTP, WebSocket, SSE, tasks, and pub/sub primitives for the Jolt framework."]

pub mod cookie;
pub mod endpoint;
pub mod endpoint_registry;
pub mod method;
pub mod request;
pub mod request_ext;
pub mod response;
pub mod router;
pub mod server;
pub mod status;

pub use cookie::Cookie;
pub use endpoint::{Endpoint, EndpointFuture};
pub use endpoint_registry::EndpointRegistry;
pub use method::{Method, ParseMethodError};
pub use request::Request;
pub use request_ext::RequestExt;
pub use response::{JsonBody, Response};
pub use router::Router;
pub use server::{CorsConfig, JoltServer, TlsConfig};
pub use status::StatusCode;

#[cfg(test)]
mod tests;
