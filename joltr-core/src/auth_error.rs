use std::fmt;

#[derive(Debug, Clone)]
pub struct AuthError {
    message: String,
}

impl AuthError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AuthError {}

impl From<&str> for AuthError {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for AuthError {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}
