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
pub struct ParseSetCookieError(String);

impl ParseSetCookieError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ParseSetCookieError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseSetCookieError {}

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

    pub fn parse(header_value: &str) -> Result<Self, ParseSetCookieError> {
        let mut parts = header_value.split(';');
        let name_value = parts
            .next()
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .ok_or_else(|| ParseSetCookieError::new("Set-Cookie header is empty"))?;

        let (name, value) = split_name_value(name_value)?;
        let mut cookie = SetCookie::new(name, value);

        for attribute in parts {
            let attribute = attribute.trim();
            if attribute.is_empty() {
                continue;
            }

            let (attribute_name, attribute_value) = split_attribute(attribute);

            if attribute_name.eq_ignore_ascii_case("Path") {
                cookie.path = Some(required_attribute_value("Path", attribute_value)?.to_string());
            } else if attribute_name.eq_ignore_ascii_case("Domain") {
                cookie.domain =
                    Some(required_attribute_value("Domain", attribute_value)?.to_string());
            } else if attribute_name.eq_ignore_ascii_case("Secure") {
                reject_flag_value("Secure", attribute_value)?;
                cookie.secure = true;
            } else if attribute_name.eq_ignore_ascii_case("HttpOnly") {
                reject_flag_value("HttpOnly", attribute_value)?;
                cookie.http_only = true;
            } else if attribute_name.eq_ignore_ascii_case("SameSite") {
                cookie.same_site = Some(parse_same_site(required_attribute_value(
                    "SameSite",
                    attribute_value,
                )?)?);
            } else if attribute_name.eq_ignore_ascii_case("Max-Age") {
                let value = required_attribute_value("Max-Age", attribute_value)?;
                cookie.max_age = Some(value.parse::<i64>().map_err(|_| {
                    ParseSetCookieError::new(format!("invalid Max-Age attribute: {value:?}"))
                })?);
            } else if attribute_name.eq_ignore_ascii_case("Expires") {
                cookie.expires =
                    Some(required_attribute_value("Expires", attribute_value)?.to_string());
            }
        }

        Ok(cookie)
    }
}

fn split_name_value(name_value: &str) -> Result<(&str, &str), ParseSetCookieError> {
    let (name, value) = name_value
        .split_once('=')
        .ok_or_else(|| ParseSetCookieError::new("Set-Cookie header is missing name=value"))?;
    let name = name.trim();

    if name.is_empty() {
        return Err(ParseSetCookieError::new("Set-Cookie cookie name is empty"));
    }

    Ok((name, value.trim()))
}

fn split_attribute(attribute: &str) -> (&str, Option<&str>) {
    match attribute.split_once('=') {
        Some((name, value)) => (name.trim(), Some(value.trim())),
        None => (attribute.trim(), None),
    }
}

fn required_attribute_value<'a>(
    name: &str,
    value: Option<&'a str>,
) -> Result<&'a str, ParseSetCookieError> {
    value
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ParseSetCookieError::new(format!("missing {name} attribute value")))
}

fn reject_flag_value(name: &str, value: Option<&str>) -> Result<(), ParseSetCookieError> {
    if value.is_some() {
        return Err(ParseSetCookieError::new(format!(
            "{name} attribute does not take a value"
        )));
    }

    Ok(())
}

fn parse_same_site(value: &str) -> Result<SameSite, ParseSetCookieError> {
    if value.eq_ignore_ascii_case("Lax") {
        Ok(SameSite::Lax)
    } else if value.eq_ignore_ascii_case("Strict") {
        Ok(SameSite::Strict)
    } else if value.eq_ignore_ascii_case("None") {
        Ok(SameSite::None)
    } else {
        Err(ParseSetCookieError::new(format!(
            "invalid SameSite attribute: {value:?}"
        )))
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
