use slab::Slab;
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

pub struct TaskScheduler {
    tasks: Slab<Box<dyn Task + Send>>,
}

impl TaskScheduler {
    pub fn new() -> Self {
        Self {
            tasks: Slab::new(),
        }
    }

    pub fn register<T: Task + Send + 'static>(&mut self, task: T) -> usize {
        self.tasks.insert(Box::new(task))
    }

    pub fn get(&self, key: usize) -> Option<&(dyn Task + Send)> {
        self.tasks.get(key).map(std::convert::AsRef::as_ref)
    }

    pub fn get_mut(&mut self, key: usize) -> Option<&mut (dyn Task + Send)> {
        let b: &mut Box<dyn Task + Send> = self.tasks.get_mut(key)?;
        Some(b.as_mut())
    }

    pub fn remove(&mut self, key: usize) -> Option<Box<dyn Task + Send>> {
        self.tasks.try_remove(key)
    }

    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    pub fn start(self) {
        for (_, mut task) in self.tasks {
            tokio::spawn(async move {
                const BACKOFF_BASE: Duration = Duration::from_secs(1);
                const BACKOFF_MAX: Duration = Duration::from_secs(60);

                let mut backoff = BACKOFF_BASE;
                loop {
                    match task.run().await {
                        Ok(()) => {
                            backoff = BACKOFF_BASE;
                            tokio::time::sleep(task.interval()).await;
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                name = task.name(),
                                backoff_secs = backoff.as_secs(),
                                "background task failed, backing off before retry"
                            );
                            tokio::time::sleep(backoff).await;
                            backoff = (backoff * 2).min(BACKOFF_MAX);
                        }
                    }
                }
            });
        }
    }
}
