//! Per-request mutable, async-shared state passed alongside [`Request`] through
//! middleware layers.
//!
//! Unlike [`Request`], whose fields are owned snapshots, `RequestExt` holds
//! state that middleware needs to flip after the request is in flight — most
//! notably the `finished` latch that signals downstream layers to skip handler
//! dispatch.
//!
//! [`Request`]: crate::Request

use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug)]
pub struct RequestExt {
    pub finished: AtomicBool,
}

impl RequestExt {
    pub fn new() -> Self {
        Self {
            finished: AtomicBool::new(false),
        }
    }

    /// Latches `finished` to `true`. Idempotent; subsequent calls are no-ops.
    pub fn mark_finished(&self) {
        self.finished.store(true, Ordering::Relaxed);
    }

    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Relaxed)
    }
}
