#![doc = "jolt-core: HTTP, WebSocket, SSE, tasks, and pub/sub primitives for the Jolt framework."]

pub mod cookie;
pub mod endpoint;
pub mod method;
pub mod request;
pub mod request_ext;
pub mod response;
pub mod server;
pub mod status;

pub use cookie::Cookie;
pub use endpoint::{Endpoint, EndpointFuture};
pub use method::{Method, ParseMethodError};
pub use request::Request;
pub use request_ext::RequestExt;
pub use response::{JsonBody, Response};
pub use server::{CorsConfig, JoltServer, TlsConfig};
pub use status::StatusCode;

#[cfg(test)]
mod tests;
