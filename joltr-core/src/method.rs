//! HTTP request method.
//!
//! [`Method`] is the canonical JoltR enum for the HTTP verbs the framework
//! routes on. Wire-format parsing (`FromStr`) is case-sensitive against the
//! uppercase canonical names defined by RFC 9110 §9.

use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Options,
    Head,
}

impl Method {
    pub fn as_str(&self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Patch => "PATCH",
            Method::Delete => "DELETE",
            Method::Options => "OPTIONS",
            Method::Head => "HEAD",
        }
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseMethodError(String);

impl fmt::Display for ParseMethodError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown HTTP method: {:?}", self.0)
    }
}

impl std::error::Error for ParseMethodError {}

impl FromStr for Method {
    type Err = ParseMethodError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "GET" => Ok(Method::Get),
            "POST" => Ok(Method::Post),
            "PUT" => Ok(Method::Put),
            "PATCH" => Ok(Method::Patch),
            "DELETE" => Ok(Method::Delete),
            "OPTIONS" => Ok(Method::Options),
            "HEAD" => Ok(Method::Head),
            other => Err(ParseMethodError(other.to_string())),
        }
    }
}

