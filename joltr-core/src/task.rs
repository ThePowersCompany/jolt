use slab::Slab;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

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
    shutdown: Arc<AtomicBool>,
    handles: Vec<JoinHandle<()>>,
}

impl TaskScheduler {
    pub fn new() -> Self {
        Self {
            tasks: Slab::new(),
            shutdown: Arc::new(AtomicBool::new(false)),
            handles: Vec::new(),
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

    pub fn shutdown_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.shutdown)
    }

    pub fn start(&mut self) {
        let shutdown = Arc::clone(&self.shutdown);

        {
            let shutdown = Arc::clone(&shutdown);
            tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                shutdown.store(true, Ordering::SeqCst);
                tracing::info!("ctrl_c received, shutdown flag set");
            });
        }

        let tasks = std::mem::take(&mut self.tasks);
        for (_, mut task) in tasks {
            let shutdown = Arc::clone(&shutdown);
            self.handles.push(tokio::spawn(async move {
                const BACKOFF_BASE: Duration = Duration::from_secs(1);
                const BACKOFF_MAX: Duration = Duration::from_secs(60);

                let mut backoff = BACKOFF_BASE;
                while !shutdown.load(Ordering::SeqCst) {
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

                tracing::info!(name = task.name(), "task loop exited via shutdown");
            }));
        }
    }

    pub async fn shutdown(&mut self, timeout: Duration) {
        self.shutdown.store(true, Ordering::SeqCst);
        tracing::info!("shutdown initiated");

        let deadline = tokio::time::Instant::now() + timeout;
        let handles = std::mem::take(&mut self.handles);

        for handle in handles {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                tracing::warn!("shutdown timeout exceeded; remaining tasks aborted");
                break;
            }
            match tokio::time::timeout(remaining, handle).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    tracing::warn!(error = %e, "task panicked during shutdown");
                }
                Err(_elapsed) => {
                    tracing::warn!("task did not stop within shutdown timeout");
                }
            }
        }
    }
}
