//! Per-request mutable, async-shared state passed alongside [`Request`] through
//! middleware layers.
//!
//! Unlike [`Request`], whose fields are owned snapshots, `RequestExt` holds
//! state that middleware needs to flip after the request is in flight — most
//! notably the `finished` latch that signals downstream layers to skip handler
//! dispatch, and an optional stashed [`Response`] that the finishing layer
//! wants the [`Router`] to surface in lieu of the matched handler.
//!
//! [`Request`]: crate::Request
//! [`Router`]: crate::Router
//! [`Response`]: axum::response::Response

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use axum::response::Response;

#[derive(Debug)]
pub struct RequestExt {
    pub finished: AtomicBool,
    /// Response stashed by a middleware layer that has called
    /// [`Self::mark_finished`]. Taken by [`crate::Router`]'s registry-driven
    /// dispatch (JOLT-RS-035) and returned in lieu of invoking the matched
    /// endpoint's handler. Behind a [`Mutex`] because [`Response`] is not
    /// [`Clone`] and the stash needs to be moved out, not copied.
    response: Mutex<Option<Response>>,
}

impl RequestExt {
    pub fn new() -> Self {
        Self {
            finished: AtomicBool::new(false),
            response: Mutex::new(None),
        }
    }

    /// Latches `finished` to `true`. Idempotent; subsequent calls are no-ops.
    pub fn mark_finished(&self) {
        self.finished.store(true, Ordering::Relaxed);
    }

    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Relaxed)
    }

    /// Stash a [`Response`] for [`crate::Router`] to surface when the
    /// `finished` latch is set. The previously-stashed response (if any) is
    /// dropped. Setting the response does NOT mark the request as finished —
    /// callers that want short-circuit behavior must also call
    /// [`Self::mark_finished`].
    pub fn set_response(&self, response: Response) {
        *self
            .response
            .lock()
            .expect("RequestExt response mutex poisoned") = Some(response);
    }

    /// Take any stashed response, leaving `None` in its place. Returns
    /// [`None`] if no response was stashed.
    pub fn take_response(&self) -> Option<Response> {
        self.response
            .lock()
            .expect("RequestExt response mutex poisoned")
            .take()
    }
}
