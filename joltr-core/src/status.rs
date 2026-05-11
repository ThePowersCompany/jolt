//! HTTP response status code.
//!
//! [`StatusCode`] enumerates the status codes JoltR routes and middleware emit
//! directly, with an `Other(u16)` catch-all so any RFC 9110 numeric code can
//! still flow through the framework.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatusCode {
    Ok,
    Created,
    NoContent,
    BadRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    MethodNotAllowed,
    Conflict,
    InternalServerError,
    Other(u16),
}

impl StatusCode {
    pub fn from_u16(code: u16) -> StatusCode {
        match code {
            200 => StatusCode::Ok,
            201 => StatusCode::Created,
            204 => StatusCode::NoContent,
            400 => StatusCode::BadRequest,
            401 => StatusCode::Unauthorized,
            403 => StatusCode::Forbidden,
            404 => StatusCode::NotFound,
            405 => StatusCode::MethodNotAllowed,
            409 => StatusCode::Conflict,
            500 => StatusCode::InternalServerError,
            other => StatusCode::Other(other),
        }
    }

    pub fn as_u16(&self) -> u16 {
        match self {
            StatusCode::Ok => 200,
            StatusCode::Created => 201,
            StatusCode::NoContent => 204,
            StatusCode::BadRequest => 400,
            StatusCode::Unauthorized => 401,
            StatusCode::Forbidden => 403,
            StatusCode::NotFound => 404,
            StatusCode::MethodNotAllowed => 405,
            StatusCode::Conflict => 409,
            StatusCode::InternalServerError => 500,
            StatusCode::Other(code) => *code,
        }
    }
}

impl From<StatusCode> for axum::http::StatusCode {
    fn from(value: StatusCode) -> Self {
        axum::http::StatusCode::from_u16(value.as_u16())
            .expect("StatusCode variants must encode valid HTTP status numbers")
    }
}

impl fmt::Display for StatusCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = self.as_u16();
        match axum::http::StatusCode::from_u16(code)
            .ok()
            .and_then(|s| s.canonical_reason())
        {
            Some(reason) => write!(f, "{} {}", code, reason),
            None => write!(f, "{}", code),
        }
    }
}

