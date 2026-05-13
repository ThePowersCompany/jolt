//! Server-Sent Events lifecycle abstractions.
//!
//! [`SseHandler`] defines the connection lifecycle (`on_open`, `on_ready`,
//! `on_close`, `on_shutdown`) with no-op defaults. [`SseEvent`] represents a
//! single SSE event with optional `event`, `id`, and `retry` fields, and
//! converts into axum's SSE event type. [`into_sse_response`] adapts a handler
//! into an `axum::response::Sse` response, drives the lifecycle, and emits
//! keep-alive comments every 15 seconds while the event stream is idle.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::response::sse::Event as AxumSseEvent;
use futures_util::stream::{empty, Stream};
use tokio::time::{self, Instant, Sleep};

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
pub trait SseHandler {
    /// Called once when a new SSE connection is opened.
    fn on_open(&mut self) -> impl Future<Output = ()> + Send {
        async {}
    }

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
    fn on_close(&mut self) -> impl Future<Output = ()> + Send {
        async {}
    }

    /// Called once after [`on_close`](Self::on_close) when the connection
    /// is fully shut down.
    fn on_shutdown(&mut self) -> impl Future<Output = ()> + Send {
        async {}
    }
}

/// Stream adapter that owns an [`SseHandler`], calls `on_close` /
/// `on_shutdown` when the inner event stream terminates or is dropped
/// (client disconnect), and interleaves keep-alive comment events when
/// the inner stream is idle.
///
/// Not part of the public API — callers go through
/// [`into_sse_response`].
struct SseCleanupStream<H: SseHandler + Send + 'static> {
    inner: SseStream,
    handler: Option<H>,
    last_event_time: Instant,
    keep_alive_interval: Duration,
    keep_alive_sleep: Pin<Box<Sleep>>,
}

impl<H: SseHandler + Send + 'static> Drop for SseCleanupStream<H> {
    fn drop(&mut self) {
        if let Some(mut handler) = self.handler.take() {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    handler.on_close().await;
                    handler.on_shutdown().await;
                });
            }
        }
    }
}

impl<H: SseHandler + Send + 'static> Stream for SseCleanupStream<H> {
    type Item = Result<AxumSseEvent, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // SAFETY: we only access `inner` / `handler` / `last_event_time` /
        // `keep_alive_interval` through &mut writes, and reset the sleep
        // via `Pin::set`. `handler.take()` is a zero-sized pointer swap.
        let this = unsafe { self.as_mut().get_unchecked_mut() };
        match Pin::new(&mut this.inner).poll_next(cx) {
            Poll::Ready(Some(event)) => {
                this.last_event_time = Instant::now();
                this.keep_alive_sleep
                    .as_mut()
                    .reset(Instant::now() + this.keep_alive_interval);
                Poll::Ready(Some(Ok(AxumSseEvent::from(event))))
            }
            Poll::Ready(None) => {
                if let Some(mut handler) = this.handler.take() {
                    if let Ok(handle) = tokio::runtime::Handle::try_current() {
                        handle.spawn(async move {
                            handler.on_close().await;
                            handler.on_shutdown().await;
                        });
                    }
                }
                Poll::Ready(None)
            }
            Poll::Pending => match this.keep_alive_sleep.as_mut().poll(cx) {
                Poll::Ready(()) => {
                    this.keep_alive_sleep
                        .as_mut()
                        .reset(Instant::now() + this.keep_alive_interval);
                    Poll::Ready(Some(Ok(AxumSseEvent::default().comment("keepalive"))))
                }
                Poll::Pending => Poll::Pending,
            },
        }
    }
}

/// Converts an [`SseHandler`] implementation into an axum SSE response
/// with a 15‑second keep‑alive comment interval.
///
/// Delegates to [`into_sse_response_with_keep_alive`] with
/// `Duration::from_secs(15)`.
///
/// ```ignore
/// async fn events_handler() -> impl IntoResponse {
///     let handler = MySseHandler::new();
///     into_sse_response(handler).await
/// }
/// ```
///
/// # Lifecycle
///
/// 1. `handler.on_open().await`
/// 2. `handler.on_ready()` stream is mapped to axum SSE events and written
///    to the response body.
/// 3. When the inner stream is idle (no events produced), a keep-alive
///    comment event (`: keepalive\\n\\n`) is emitted every 15 seconds to
///    prevent proxy timeouts.
/// 4. When the stream ends (naturally) or the client disconnects,
///    `handler.on_close()` and `handler.on_shutdown()` fire in a
///    background task.
pub async fn into_sse_response<H: SseHandler + Send + 'static>(
    handler: H,
) -> axum::response::Sse<impl Stream<Item = Result<AxumSseEvent, Infallible>>> {
    into_sse_response_with_keep_alive(handler, Duration::from_secs(15)).await
}

/// Converts an [`SseHandler`] implementation into an axum SSE response
/// with a configurable keep‑alive comment interval.
///
/// Same lifecycle as [`into_sse_response`], but `keep_alive` controls how
/// often a `: keepalive\\n\\n` comment is sent when the inner stream is
/// idle.
pub async fn into_sse_response_with_keep_alive<H: SseHandler + Send + 'static>(
    mut handler: H,
    keep_alive: Duration,
) -> axum::response::Sse<impl Stream<Item = Result<AxumSseEvent, Infallible>>> {
    handler.on_open().await;
    let inner = handler.on_ready();
    let now = Instant::now();
    axum::response::Sse::new(SseCleanupStream {
        inner,
        handler: Some(handler),
        last_event_time: now,
        keep_alive_interval: keep_alive,
        keep_alive_sleep: Box::pin(time::sleep(keep_alive)),
    })
}
