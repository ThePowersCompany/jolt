//! Per-request mutable, async-shared state passed alongside [`Request`] through
//! middleware layers.
//!
//! Unlike [`Request`], whose fields are owned snapshots, `RequestExt` holds
//! state that middleware needs to flip after the request is in flight — most
//! notably the `finished` latch that signals downstream layers to skip handler
//! dispatch.
//!
//! [`Request`]: crate::Request

use std::sync::atomic::AtomicBool;

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
}
