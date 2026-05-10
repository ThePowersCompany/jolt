//! HTTP response status code.
//!
//! [`StatusCode`] enumerates the status codes Jolt routes and middleware emit
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_u16_known_is_named() {
        assert_eq!(StatusCode::from_u16(200), StatusCode::Ok);
    }

    #[test]
    fn from_u16_unknown_is_other() {
        assert_eq!(StatusCode::from_u16(418), StatusCode::Other(418));
    }

    #[test]
    fn into_axum_status_not_found() {
        let axum_status: axum::http::StatusCode = StatusCode::NotFound.into();
        assert_eq!(axum_status, axum::http::StatusCode::NOT_FOUND);
        assert_eq!(axum_status.as_u16(), 404);
    }

    #[test]
    fn display_known_includes_reason_phrase() {
        assert_eq!(StatusCode::NotFound.to_string(), "404 Not Found");
        assert_eq!(StatusCode::Ok.to_string(), "200 OK");
    }

    #[test]
    fn display_other_with_known_code_uses_reason_phrase() {
        assert_eq!(StatusCode::Other(418).to_string(), "418 I'm a teapot");
    }
}
