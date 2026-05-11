use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct TaskError {
    message: String,
}

impl TaskError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for TaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TaskError {}

impl From<&str> for TaskError {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for TaskError {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

pub type TaskFuture<'a> = Pin<Box<dyn Future<Output = Result<(), TaskError>> + Send + 'a>>;

pub trait Task {
    fn name(&self) -> &str;

    fn interval(&self) -> Duration;

    fn run(&mut self) -> TaskFuture<'_>;
}
