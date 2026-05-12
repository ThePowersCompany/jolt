//! Cookie value types used by requests and responses.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SameSite {
    Lax,
    Strict,
    None,
}

impl fmt::Display for SameSite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SameSite::Lax => f.write_str("Lax"),
            SameSite::Strict => f.write_str("Strict"),
            SameSite::None => f.write_str("None"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetCookie {
    pub name: String,
    pub value: String,
    pub path: Option<String>,
    pub domain: Option<String>,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: Option<SameSite>,
    pub max_age: Option<i64>,
    pub expires: Option<String>,
}

impl SetCookie {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            path: None,
            domain: None,
            secure: false,
            http_only: false,
            same_site: None,
            max_age: None,
            expires: None,
        }
    }

    pub fn path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    pub fn domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = Some(domain.into());
        self
    }

    pub fn secure(mut self) -> Self {
        self.secure = true;
        self
    }

    pub fn http_only(mut self) -> Self {
        self.http_only = true;
        self
    }

    pub fn same_site(mut self, same_site: SameSite) -> Self {
        self.same_site = Some(same_site);
        self
    }

    pub fn max_age(mut self, max_age: i64) -> Self {
        self.max_age = Some(max_age);
        self
    }

    pub fn expires(mut self, expires: impl Into<String>) -> Self {
        self.expires = Some(expires.into());
        self
    }

    pub fn to_header(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for SetCookie {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}={}", self.name, self.value)?;

        if let Some(path) = &self.path {
            write!(f, "; Path={path}")?;
        }

        if let Some(domain) = &self.domain {
            write!(f, "; Domain={domain}")?;
        }

        if self.secure {
            f.write_str("; Secure")?;
        }

        if self.http_only {
            f.write_str("; HttpOnly")?;
        }

        if let Some(same_site) = self.same_site {
            write!(f, "; SameSite={same_site}")?;
        }

        if let Some(max_age) = self.max_age {
            write!(f, "; Max-Age={max_age}")?;
        }

        if let Some(expires) = &self.expires {
            write!(f, "; Expires={expires}")?;
        }

        Ok(())
    }
}
