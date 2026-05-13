//! Unified error abstraction for endpoint handlers.
//!
//! [`JoltRError`] is the trait a typed endpoint error implements so that a
//! handler returning `Result<Response<T>, E> where E: JoltRError` can surface
//! the failure to the client as a JSON response with shape
//! `{ "error": <message>, "status": <code> }`. The trait carries default
//! conversion logic (via [`JoltRError::to_response`]) so most implementors
//! only need to spell out `status()` and `message()`.
//!
//! The companion JSON body shape is exposed as [`ErrorBody`]; it implements
//! [`JsonBody`] so the existing `Response<T> -> axum::response::Response`
//! bridge serializes it as `application/json` without any extra glue.

use serde::Serialize;

use crate::response::{JsonBody, Response};
use crate::status::StatusCode;

/// JSON body shape used by [`JoltRError::to_response`]: an `error` message
/// string and the numeric `status` code, matching the contract documented in
/// the PRD.
#[derive(Debug, Clone, Serialize)]
pub struct ErrorBody {
    pub error: String,
    pub status: u16,
}

impl JsonBody for ErrorBody {}

/// Trait implemented by typed endpoint errors so they can be converted into
/// JSON error responses with shape `{ "error": <message>, "status": <code> }`.
///
/// The `#[endpoint]` codegen consumes this trait to wrap handler results of
/// the form `Result<Response<T>, E>`: on `Err(e)`, it calls
/// `e.to_response()` and forwards the resulting `Response<ErrorBody>` through
/// the existing `From<Response<T>> for axum::response::Response` bridge.
pub trait JoltRError {
    /// HTTP status code that should accompany this error.
    fn status(&self) -> StatusCode;

    /// Human-readable error message surfaced under the `error` JSON key.
    fn message(&self) -> String;

    /// Build the JSON-shaped [`Response<ErrorBody>`] for this error. The
    /// default implementation is sufficient for the contract in the PRD;
    /// implementors only override it if they need to attach extra headers
    /// (e.g. `WWW-Authenticate` on a 401) or use a non-default body layout.
    fn to_response(&self) -> Response<ErrorBody> {
        let status = self.status();
        Response::new(
            status,
            ErrorBody {
                error: self.message(),
                status: status.as_u16(),
            },
        )
    }
}
