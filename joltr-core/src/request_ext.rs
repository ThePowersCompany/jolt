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

use std::any::Any;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::Request as AxumRequest;
use axum::http::StatusCode;
use axum::response::Response;

#[derive(Debug)]
pub struct RequestExt {
    pub finished: AtomicBool,
    /// Response stashed by a middleware layer that has called
    /// [`Self::mark_finished`]. Taken by [`crate::Router`]'s registry-driven
    /// dispatch (JOLTR-RS-035) and returned in lieu of invoking the matched
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

#[doc(hidden)]
pub fn take_finished_response_for<Req: Any>(req: &Req) -> Option<Response> {
    let axum_req = (req as &dyn Any).downcast_ref::<AxumRequest>()?;
    let request_ext = axum_req.extensions().get::<Arc<RequestExt>>()?;

    if request_ext.is_finished() {
        Some(take_short_circuit_response(request_ext))
    } else {
        None
    }
}

#[doc(hidden)]
pub fn take_short_circuit_response(ext: &Arc<RequestExt>) -> Response {
    ext.take_response().unwrap_or_else(|| {
        Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::empty())
            .expect("static 500 builder always succeeds")
    })
}
