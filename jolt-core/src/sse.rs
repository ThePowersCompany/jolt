//! Server-Sent Events lifecycle abstractions (phase32-sse).
//!
//! Phase32 ladder:
//! - JOLT-RS-135: define the [`SseHandler`] trait with four lifecycle
//!   methods (`on_open`, `on_ready` -> stream, `on_close`, `on_shutdown`)
//!   and no-op default impls. Define a minimal placeholder shape for
//!   [`SseEvent`] so the trait signatures can reference it.
//! - JOLT-RS-136: flesh out [`SseEvent`] with `name`, `id`, `retry`
//!   optional fields and wire `From<SseEvent> for axum::response::sse::Event`.
//! - JOLT-RS-137: SSE endpoint adapter that creates `axum::response::Sse`
//!   from the handler's `on_ready()` stream.
//! - JOLT-RS-138: keep-alive comment events every 15s to prevent proxy
//!   timeouts.

use std::pin::Pin;
use std::time::Duration;

use axum::response::sse::Event as AxumSseEvent;
use futures_util::stream::{empty, Stream};

/// A single SSE event.
///
/// Fields mirror the SSE specification (whatwg § 9.2):
///
/// | Field   | SSE wire    | Type            |
/// |---------|-------------|-----------------|
/// | `name`  | `event:`    | `Option<String>`|
/// | `data`  | `data:`     | `String`        |
/// | `id`    | `id:`       | `Option<String>`|
/// | `retry` | `retry:`    | `Option<u64>`   |
///
/// # Conversion to axum SSE
///
/// `From<SseEvent> for `[`AxumSseEvent`]` maps each field:
/// - `data` → `.data(self.data)`
/// - `name` (if `Some`) → `.event(name)`
/// - `id` (if `Some`) → `.id(id)`
/// - `retry` (if `Some`) → `.retry(Duration::from_millis(self.retry))`
pub struct SseEvent {
    /// Event type name (`event:` line). If `None`, the `event:` line is omitted.
    pub name: Option<String>,
    /// The event payload text. Required for every SSE event.
    pub data: String,
    /// Event identifier (`id:` line). If `None`, the `id:` line is omitted.
    pub id: Option<String>,
    /// Reconnection time hint in milliseconds (`retry:` line). If `None`,
    /// the `retry:` line is omitted.
    pub retry: Option<u64>,
}

impl SseEvent {
    /// Convenience constructor for the common case: an SSE event with a
    /// named event type and payload data, with no `id` or `retry` hints.
    pub fn new(name: &str, data: &str) -> Self {
        Self {
            name: Some(name.to_owned()),
            data: data.to_owned(),
            id: None,
            retry: None,
        }
    }
}

impl From<SseEvent> for AxumSseEvent {
    fn from(e: SseEvent) -> Self {
        let mut event = AxumSseEvent::default().data(e.data);
        if let Some(name) = e.name {
            event = event.event(name);
        }
        if let Some(id) = e.id {
            event = event.id(id);
        }
        if let Some(retry_ms) = e.retry {
            event = event.retry(Duration::from_millis(retry_ms));
        }
        event
    }
}

/// Boxed, pinned stream of [`SseEvent`] — the return type of
/// [`SseHandler::on_ready`]. Mirrors the [`EndpointFuture`](crate::EndpointFuture)
/// type-alias pattern used by the [`Endpoint`](crate::Endpoint) trait.
pub type SseStream = Pin<Box<dyn Stream<Item = SseEvent> + Send>>;

/// Trait implemented by user-defined SSE handler structs.
///
/// Each lifecycle method carries a default no-op implementation so handlers
/// that only care about the event stream can override just [`on_ready`](Self::on_ready).
///
/// # Example
///
/// ```ignore
/// struct ClockHandler;
///
/// impl SseHandler for ClockHandler {
///     fn on_ready(&mut self) -> SseStream {
///         let stream = tokio_stream::wrappers::IntervalStream::new(
///             tokio::time::interval(Duration::from_secs(1)),
///         )
///         .map(|_| SseEvent::new("tick", &chrono::Utc::now().to_string()));
///         Box::pin(stream)
///     }
/// }
/// ```
#[allow(async_fn_in_trait)]
pub trait SseHandler {
    /// Called once when a new SSE connection is opened.
    async fn on_open(&mut self) {}

    /// Called after [`on_open`](Self::on_open). Returns the stream of
    /// [`SseEvent`] items that will be written to the SSE connection.
    /// The server drives this stream until it ends or the client disconnects.
    /// The default implementation returns an empty stream that terminates
    /// immediately.
    fn on_ready(&mut self) -> SseStream {
        Box::pin(empty())
    }

    /// Called once when the SSE client disconnects or the connection is
    /// torn down.
    async fn on_close(&mut self) {}

    /// Called once after [`on_close`](Self::on_close) when the connection
    /// is fully shut down.
    async fn on_shutdown(&mut self) {}
}
