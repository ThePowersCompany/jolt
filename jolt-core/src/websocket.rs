//! WebSocket lifecycle abstractions (phase28-ws-trait).
//!
//! Phase28 ladder:
//! - JOLT-RS-118: define the [`WebSocketHandler`] trait with five async
//!   lifecycle methods (`on_open`, `on_ready`, `on_message`, `on_close`,
//!   `on_shutdown`) and no-op default impls. Define minimal placeholder
//!   shapes for [`WebSocketSender`] and [`WsMessage`] so the trait
//!   signatures can reference them.
//! - JOLT-RS-119: flesh out [`WebSocketSender`] with the
//!   `send` / `send_text` / `send_json` / `close` methods, an mpsc-backed
//!   transport so the type is cheaply [`Clone`]able (decision 5 from 118),
//!   a constructor [`WebSocketSender::channel`] that returns the sender +
//!   the writer-task feeder receiver, and the [`WsSendError`] enum.
//! - JOLT-RS-120 (this iteration): replace the empty [`WsMessage`] enum with
//!   `Text(String)`, `Binary(Vec<u8>)`, `Ping(Vec<u8>)`, `Pong(Vec<u8>)`, and
//!   `Close(Option<CloseFrame>)` variants; introduce a Jolt-owned
//!   [`CloseFrame`] (`code: u16`, `reason: String`) so the public surface
//!   doesn't expose axum's lifetime-parameterized `CloseFrame<'static>`;
//!   wire up `From<axum::extract::ws::Message> for WsMessage` and the inverse
//!   so 124's read/write loops can convert between the framework and axum
//!   layers; replace [`WebSocketSender::send`]'s `match msg {}` body with a
//!   real conversion that dispatches the corresponding axum frame.
//! - JOLT-RS-121: write the lifecycle-callback ordering unit test
//!   (open → ready → message → close fires in order on a mock impl).
//! - JOLT-RS-122..125: the `ws!` macro that registers a handler against an
//!   axum router behind a JWT-subprotocol auth guard (consumes the trait
//!   declared here, composes with [`AuthWsJwtLayer`](crate::AuthWsJwtLayer)
//!   from JOLT-RS-076).
//!
//! Architectural decisions pinned here for 120..125 to build on:
//!
//! 1. **Native `async fn` in traits, not the `async-trait` crate.** Rust 1.75+
//!    natively supports `async fn` in trait declarations (RPITIT); the
//!    workspace's `cargo 1.95` and edition 2021 satisfy the requirement.
//!    Avoiding the `async-trait` macro keeps the trait's method signatures
//!    legible (no `Pin<Box<dyn Future + Send>>` boxing) and removes a runtime
//!    allocation per call. The trade-off is that the futures produced by the
//!    methods carry no auto-trait bounds (notably `Send`) by default — the
//!    `#[allow(async_fn_in_trait)]` on the trait silences the in-trait lint
//!    that flags this. 124's WS-upgrade handler will express the needed
//!    `Send` constraint at the call site via a bound like
//!    `for<'a> H::on_open(&'a mut H, _): Send` (return-type notation,
//!    stabilized in 1.79) rather than via a trait-level attribute, so a
//!    single-threaded executor can still implement the trait without a
//!    needless `Send` requirement.
//!
//! 2. **All five methods default to no-ops.** Per the PRD ("All have default
//!    no-op impls"), implementing the trait on a unit struct must compile
//!    without overriding any method. The intent is that handlers opt into
//!    only the lifecycle stages they care about — a simple echo handler
//!    overrides `on_message`; a chat handler also overrides `on_open` to
//!    subscribe to a pub/sub channel; a heartbeat handler also overrides
//!    `on_ready` to spawn a ping timer. This mirrors facil.io's `ws_handler_s`
//!    callback table where every slot is nullable (see `spec_rust.md` §"Phase
//!    4 — Real-time: WebSockets…").
//!
//! 3. **[`WebSocketSender`] is an mpsc-backed handle to a writer task that
//!    owns the axum sink.** 119 made the type cheaply [`Clone`]able by
//!    wrapping a [`tokio::sync::mpsc::UnboundedSender`] of [`axum::extract::ws::Message`]
//!    rather than locking the sink directly. The framework constructs the
//!    pair via [`WebSocketSender::channel`]; 124 will spawn a writer task
//!    that consumes the returned receiver and forwards each frame into the
//!    axum sink half. Decision 5 (the sender is passed by value to handler
//!    callbacks so handlers can move it into spawned tasks) only works if
//!    cloning is cheap — the mpsc handle is one `Arc` clone, no mutex
//!    contention. [`WsMessage`] is still the public-facing variant set;
//!    the internal axum [`Message`] is an implementation detail not
//!    exposed on the public API.
//!
//! 4. **Receiver is `&mut self` across the board.** Lifecycle handlers
//!    routinely mutate per-connection state (subscription set, message
//!    counters, last-pong instant). Forcing `&self` would push implementers
//!    onto interior mutability (`Mutex` / `RefCell`) for the common case.
//!    `&mut self` matches the call pattern in 124's generated upgrade
//!    handler, which owns the handler instance and drives it sequentially
//!    through the lifecycle (no concurrent calls on a single handler).
//!
//! 5. **`WebSocketSender` is passed by value to the open/ready/message
//!    methods, and `on_close` / `on_shutdown` take no sender.** 119 makes
//!    `WebSocketSender` cheaply clonable (it wraps an [`Arc`]-backed mpsc
//!    sender to a writer task), so passing by value is no more expensive
//!    than passing `&WebSocketSender` and lets handlers move the sender into
//!    spawned tasks (`tokio::spawn`-and-forget for pub/sub fan-in,
//!    heartbeats, etc.) without lifetime gymnastics. `on_close` and
//!    `on_shutdown` take no sender — the socket is being torn down; sending
//!    into it is a no-op at best.
//!
//! 6. **No trait-level `Error` associated type.** The PRD-118 signatures are
//!    all `async fn ... -> ()`; lifecycle errors are the handler's
//!    responsibility to log / surface internally. 119's
//!    [`WebSocketSender::send_text`] / [`send_json`](WebSocketSender::send_json)
//!    return [`Result<(), WsSendError>`], but those errors don't propagate
//!    up the trait — the handler decides how to react (drop the connection,
//!    retry, log). Adding a trait-level `Error` associated type would force
//!    every handler to declare one, even for the common case where the
//!    handler swallows all errors internally.

use axum::extract::ws::Message as AxumMessage;
use serde::Serialize;
use std::fmt;
use tokio::sync::mpsc;

/// Server-side half of a WebSocket connection. Passed by value into
/// [`WebSocketHandler::on_open`], [`WebSocketHandler::on_ready`], and
/// [`WebSocketHandler::on_message`].
///
/// Cheaply [`Clone`]able — internally an [`Arc`](std::sync::Arc)-backed
/// mpsc handle (the writer task owns the axum sink and consumes frames
/// posted to this channel). Handlers can clone the sender and move it into
/// spawned tasks (pub/sub fan-in, heartbeats) without lifetime gymnastics
/// — decision 5 in the module docs.
///
/// `#[non_exhaustive]` here prevents downstream pattern destructuring with
/// struct-literal syntax; the type's internal shape is reserved for future
/// changes (e.g. swapping the unbounded channel for a bounded one if
/// back-pressure becomes a concern).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct WebSocketSender {
    tx: mpsc::UnboundedSender<AxumMessage>,
}

/// Error returned by the [`WebSocketSender`] send / close methods.
///
/// Per decision 6, these errors don't propagate up the [`WebSocketHandler`]
/// trait — the handler decides whether to log, retry, or abort. The two
/// variants correspond to the two ways an outbound write can fail: the
/// channel to the writer task is closed (connection torn down), or the
/// value passed to [`WebSocketSender::send_json`] could not be serialized.
#[derive(Debug)]
pub enum WsSendError {
    /// The underlying WebSocket writer task has dropped its receiver — the
    /// connection is either closed or being torn down. No further sends
    /// will succeed on this sender.
    Closed,
    /// [`WebSocketSender::send_json`] failed to serialize the value via
    /// `serde_json`. The send was never attempted.
    Serialize(serde_json::Error),
}

impl fmt::Display for WsSendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => f.write_str("websocket sender channel is closed"),
            Self::Serialize(e) => write!(f, "failed to serialize value to JSON: {e}"),
        }
    }
}

impl std::error::Error for WsSendError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Closed => None,
            Self::Serialize(e) => Some(e),
        }
    }
}

impl WebSocketSender {
    /// Constructs a sender + writer-task feeder receiver pair. The
    /// framework's WS upgrade flow (124) calls this once per accepted
    /// connection, spawns a writer task that consumes the returned
    /// [`mpsc::UnboundedReceiver`] and forwards each frame to the axum
    /// sink half, and hands clones of the sender to the
    /// [`WebSocketHandler`] callbacks. The unbounded channel pairs with
    /// the axum sink's natural back-pressure (a slow writer task will pile
    /// up frames in memory; bounded channels are a follow-up if that
    /// becomes a real concern under load).
    pub fn channel() -> (Self, mpsc::UnboundedReceiver<AxumMessage>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self { tx }, rx)
    }

    /// Sends a typed [`WsMessage`] frame.
    ///
    /// Converts to the underlying [`AxumMessage`] via [`From<WsMessage>`]
    /// (lossless across the five variants) and dispatches through the same
    /// channel as the typed helpers. Returns [`WsSendError::Closed`] if the
    /// writer-task receiver has been dropped.
    pub fn send(&self, msg: WsMessage) -> Result<(), WsSendError> {
        self.dispatch(AxumMessage::from(msg))
    }

    /// Sends a UTF-8 text frame.
    ///
    /// Returns [`WsSendError::Closed`] if the connection's writer task has
    /// dropped its receiver.
    pub fn send_text(&self, text: &str) -> Result<(), WsSendError> {
        self.dispatch(AxumMessage::Text(text.to_string()))
    }

    /// Serializes `val` to JSON via `serde_json` and sends it as a UTF-8
    /// text frame.
    ///
    /// Returns [`WsSendError::Serialize`] if serialization fails (no send
    /// is attempted in that case), or [`WsSendError::Closed`] if the
    /// connection's writer task has dropped its receiver.
    pub fn send_json(&self, val: &impl Serialize) -> Result<(), WsSendError> {
        let text = serde_json::to_string(val).map_err(WsSendError::Serialize)?;
        self.dispatch(AxumMessage::Text(text))
    }

    /// Sends a close frame with no payload. Subsequent `send_*` calls on
    /// this or any clone of this sender will return [`WsSendError::Closed`]
    /// once the writer task observes the close and drops its receiver.
    pub fn close(&self) -> Result<(), WsSendError> {
        self.dispatch(AxumMessage::Close(None))
    }

    /// Internal channel dispatch shared by [`send_text`](Self::send_text),
    /// [`send_json`](Self::send_json), and [`close`](Self::close). Maps the
    /// channel's send-error into the public [`WsSendError`] surface — the
    /// underlying [`mpsc::error::SendError`] carries the un-sent message,
    /// which we deliberately drop here (the public surface doesn't expose
    /// axum's [`Message`](AxumMessage) type per decision 3).
    fn dispatch(&self, msg: AxumMessage) -> Result<(), WsSendError> {
        self.tx.send(msg).map_err(|_| WsSendError::Closed)
    }
}

/// A close-handshake frame attached to [`WsMessage::Close`].
///
/// This is a Jolt-owned newtype rather than a re-export of
/// [`axum::extract::ws::CloseFrame`], for two reasons:
///
/// 1. axum's `CloseFrame<'t>` is lifetime-parameterized (its `reason` is
///    `Cow<'t, str>`). Propagating that lifetime through [`WsMessage`] would
///    force every handler signature to either pin a `'static` argument or
///    add a lifetime parameter to the trait. Owning a [`String`] here keeps
///    [`WsMessage`] lifetime-free; the conversion to axum's frame allocates
///    a `Cow::Owned` on the way out.
/// 2. Insulating the public surface from axum's frame shape lets us add
///    fields (a Jolt-side `code` enum, a trailing diagnostic string) in a
///    future slice without forcing downstream code to track axum upgrades.
///
/// The `code` is a raw `u16` matching axum's [`CloseCode`](axum::extract::ws::CloseCode)
/// (also `pub type CloseCode = u16`) — RFC 6455 §7.4 close codes (`1000`
/// normal closure, `1011` internal error, etc.). A higher-level Jolt close
/// code enum is deferred.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CloseFrame {
    /// RFC 6455 close code (e.g. `1000` for normal closure).
    pub code: u16,
    /// Human-readable close reason. May be empty.
    pub reason: String,
}

/// Inbound or outbound WebSocket frame.
///
/// Delivered to [`WebSocketHandler::on_message`] on the inbound side and
/// accepted by [`WebSocketSender::send`] on the outbound side. The five
/// variants match RFC 6455 frame types as exposed by axum's
/// [`Message`](AxumMessage); the mapping is lossless via the
/// [`From<AxumMessage>`] / [`From<WsMessage>`] impls below.
///
/// `#[non_exhaustive]` documents that future variant additions (e.g. a
/// fragmented-frame variant) remain non-breaking: downstream `match` arms
/// must always include a `_ =>` wildcard.
#[derive(Debug, Clone, Eq, PartialEq)]
#[non_exhaustive]
pub enum WsMessage {
    /// UTF-8 text frame.
    Text(String),
    /// Opaque binary payload.
    Binary(Vec<u8>),
    /// Control-frame ping with an arbitrary payload (≤ 125 bytes per RFC).
    /// axum auto-replies to pings; handlers usually observe these for
    /// liveness bookkeeping rather than responding.
    Ping(Vec<u8>),
    /// Control-frame pong with an arbitrary payload (≤ 125 bytes per RFC).
    /// Emitted in response to a ping (handled automatically by axum) or as
    /// an unsolicited unidirectional heartbeat.
    Pong(Vec<u8>),
    /// Close handshake frame. `None` signals "close without payload";
    /// `Some(_)` carries the peer's code + reason.
    Close(Option<CloseFrame>),
}

impl From<AxumMessage> for WsMessage {
    /// Converts an inbound axum frame into the Jolt-side [`WsMessage`].
    /// Lossless across all five variants: text / binary / ping / pong pass
    /// their payloads through verbatim, and the close frame's `Cow` reason
    /// is materialized into an owned [`String`].
    fn from(msg: AxumMessage) -> Self {
        match msg {
            AxumMessage::Text(s) => Self::Text(s),
            AxumMessage::Binary(b) => Self::Binary(b),
            AxumMessage::Ping(p) => Self::Ping(p),
            AxumMessage::Pong(p) => Self::Pong(p),
            AxumMessage::Close(Some(frame)) => Self::Close(Some(CloseFrame {
                code: frame.code,
                reason: frame.reason.into_owned(),
            })),
            AxumMessage::Close(None) => Self::Close(None),
        }
    }
}

impl From<WsMessage> for AxumMessage {
    /// Converts an outbound [`WsMessage`] back into axum's frame type. The
    /// close-frame `reason` becomes a `Cow::Owned`, so the resulting
    /// `CloseFrame<'static>` doesn't borrow from the input.
    fn from(msg: WsMessage) -> Self {
        match msg {
            WsMessage::Text(s) => AxumMessage::Text(s),
            WsMessage::Binary(b) => AxumMessage::Binary(b),
            WsMessage::Ping(p) => AxumMessage::Ping(p),
            WsMessage::Pong(p) => AxumMessage::Pong(p),
            WsMessage::Close(Some(frame)) => AxumMessage::Close(Some(
                axum::extract::ws::CloseFrame {
                    code: frame.code,
                    reason: std::borrow::Cow::Owned(frame.reason),
                },
            )),
            WsMessage::Close(None) => AxumMessage::Close(None),
        }
    }
}

/// Lifecycle callbacks driven by a WebSocket connection. All five methods
/// default to no-ops, so implementers override only the stages they care
/// about.
///
/// The callbacks fire in this order on a normal connection:
/// `on_open` → `on_ready` → (`on_message` repeated 0..N times) → `on_close` →
/// `on_shutdown`. JOLT-RS-124's generated upgrade handler drives the
/// sequence; JOLT-RS-121's test pins the ordering against a mock impl.
///
/// See the module-level docs for the rationale on native `async fn`,
/// `&mut self`, the by-value sender, and the absence of a trait-level
/// `Error` associated type.
///
/// # Example
///
/// ```ignore
/// use jolt_core::{WebSocketHandler, WebSocketSender, WsMessage};
///
/// struct EchoHandler;
///
/// impl WebSocketHandler for EchoHandler {
///     async fn on_open(&mut self, sender: WebSocketSender) {
///         let _ = sender.send_text("welcome");
///     }
///
///     async fn on_message(&mut self, _msg: WsMessage, _sender: WebSocketSender) {
///         // 120 adds WsMessage variants; at 119 this body is unreachable.
///     }
/// }
/// ```
#[allow(async_fn_in_trait)]
pub trait WebSocketHandler {
    /// Called once immediately after the WebSocket upgrade completes, before
    /// any messages are read. Use this hook to register subscriptions or
    /// stash per-connection state (claims, user id, etc.).
    async fn on_open(&mut self, _sender: WebSocketSender) {}

    /// Called once after [`on_open`](Self::on_open) returns, signaling the
    /// connection is fully wired and the read loop is about to start. Use
    /// this hook to send an initial greeting frame, spawn timers, etc.
    async fn on_ready(&mut self, _sender: WebSocketSender) {}

    /// Called for each inbound frame. May fire zero or more times across the
    /// connection's lifetime. The default implementation drops the message.
    async fn on_message(&mut self, _msg: WsMessage, _sender: WebSocketSender) {}

    /// Called once when the peer initiates a close (or the read loop exits
    /// due to an I/O error). No sender is passed because the socket is being
    /// torn down. Use this hook to flush state or remove pub/sub
    /// subscriptions.
    async fn on_close(&mut self) {}

    /// Called once after [`on_close`](Self::on_close) when the connection is
    /// fully shut down (writer task joined). Use this hook for terminal
    /// bookkeeping that must run after all I/O has settled.
    async fn on_shutdown(&mut self) {}
}

/// Witness value returned by the `ws!` macro at JOLT-RS-122 (phase29 opener).
///
/// The macro emits a block expression that constructs this struct with the
/// two string-literal arguments (`path`, `subprotocol`) so a downstream
/// integration test can observe that the macro parsed and expanded
/// correctly. The two remaining macro arguments (the handler type and the
/// auth-fn path) don't carry runtime values, so they're type-checked at
/// compile time inside the same expansion via a `const _: fn() = ...`
/// trait-bound closure (handler type) and a `let _ = ...;` (auth_fn path).
///
/// JOLT-RS-124 will replace this witness with the real WS upgrade-handler
/// return type (likely an [`axum::Router`] fragment or a registration
/// helper); the witness exists only to give 122 a typed return value to
/// land before the upgrade-handler codegen does.
///
/// `#[doc(hidden)]` and the `__` prefix mark this as not part of the stable
/// public API — user code should never reference `__WsMacroWitness` directly.
#[doc(hidden)]
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct __WsMacroWitness {
    /// The path string-literal passed to `ws!` as the first positional
    /// argument (e.g. `"/chat"`).
    pub path: &'static str,
    /// The subprotocol string-literal passed to `ws!` as the
    /// `subprotocol = "..."` named argument (e.g. `"chat-v1"`).
    pub subprotocol: &'static str,
}

#[cfg(test)]
mod tests {
    //! Phase28 test bundle:
    //!
    //! - JOLT-RS-118 tests (preserved): three witnesses for the
    //!   [`WebSocketHandler`] trait — (a) a unit struct can implement it
    //!   with zero overrides, (b) partial overrides satisfy the trait
    //!   bound, (c) the default impls execute to completion.
    //! - JOLT-RS-119 tests (preserved): five witnesses for [`WebSocketSender`]
    //!   — `send_text` dispatches an [`AxumMessage::Text`] frame through the
    //!   channel, `send_json` serializes-then-dispatches, `close` dispatches
    //!   [`AxumMessage::Close(None)`](AxumMessage::Close), sending after the
    //!   writer-task receiver is dropped yields [`WsSendError::Closed`], and
    //!   a clone of a sender shares the same channel (cheap-clone witness
    //!   for decision 5).
    //! - JOLT-RS-120 tests (new): witnesses for the [`WsMessage`] variants
    //!   and the axum mapping — (a) `axum Text → WsMessage::Text` (the PRD
    //!   verification), (b) the same for binary / ping / pong / close(None)
    //!   / close(Some), (c) the inverse `WsMessage → AxumMessage` mapping,
    //!   (d) [`WebSocketSender::send`] now dispatches typed frames through
    //!   the channel for each variant, and (e) close-frame round-trip
    //!   preserves the code and reason.

    use super::*;

    /// A minimal handler that overrides nothing. Witnesses claim (1):
    /// `WebSocketHandler` is implementable with all-default methods.
    struct NoOverrideHandler;
    impl WebSocketHandler for NoOverrideHandler {}

    /// A handler that overrides on_open and on_message but leaves the other
    /// three at their defaults. Witnesses claim (2): partial overrides
    /// compose with default methods.
    struct PartialOverrideHandler {
        open_called: bool,
    }
    impl WebSocketHandler for PartialOverrideHandler {
        async fn on_open(&mut self, _sender: WebSocketSender) {
            self.open_called = true;
        }
        async fn on_message(&mut self, _msg: WsMessage, _sender: WebSocketSender) {
            // The 118/119 tests only exercise the open/ready/close/shutdown
            // path; on_message stays a panic-on-call witness so an
            // accidentally-routed message would surface loudly. 121's
            // lifecycle-ordering test will replace this with call-order
            // recording across all five callbacks.
            unreachable!("PartialOverrideHandler.on_message is not exercised by 118-120 tests");
        }
    }

    #[test]
    fn no_override_compiles_with_defaults() {
        // Compile-time witness: takes any T: WebSocketHandler, so the test
        // body's call form forces the trait bound to resolve. A regression
        // that removed a default impl would fail to compile here, not at
        // runtime.
        fn assert_impls_handler<T: WebSocketHandler>() {}
        assert_impls_handler::<NoOverrideHandler>();
    }

    #[test]
    fn partial_override_satisfies_trait_bound() {
        fn assert_impls_handler<T: WebSocketHandler>() {}
        assert_impls_handler::<PartialOverrideHandler>();
    }

    #[tokio::test]
    async fn default_impls_run_to_completion_on_unit_struct_handler() {
        // Runtime witness: each default impl returns `()` and doesn't
        // panic. on_message is omitted because WsMessage is uninhabited at
        // 118/119; 121 covers it once 120 lands the variants.
        let (sender, _rx) = WebSocketSender::channel();
        let mut handler = NoOverrideHandler;
        handler.on_open(sender.clone()).await;
        handler.on_ready(sender).await;
        handler.on_close().await;
        handler.on_shutdown().await;
    }

    #[tokio::test]
    async fn partial_override_fires_overridden_on_open_and_leaves_others_default() {
        // Confirms that overriding on_open doesn't accidentally shadow the
        // sibling defaults — on_close and on_shutdown still run as no-ops.
        let (sender, _rx) = WebSocketSender::channel();
        let mut handler = PartialOverrideHandler { open_called: false };
        handler.on_open(sender.clone()).await;
        assert!(handler.open_called, "on_open override must have fired");
        handler.on_ready(sender).await;
        handler.on_close().await;
        handler.on_shutdown().await;
    }

    #[tokio::test]
    async fn send_text_dispatches_text_frame_through_underlying_channel() {
        // PRD-119 verification: "sender.send_text(\"hi\") calls underlying
        // WS send." The mpsc receiver IS the writer task's view of the
        // underlying sink at 119 (the spawned task forwarding rx → axum
        // sink lands at 124); receiving the frame on rx proves the call
        // landed on the writer-task feeder channel.
        let (sender, mut rx) = WebSocketSender::channel();
        sender
            .send_text("hi")
            .expect("send_text on an open sender should succeed");
        let msg = rx
            .recv()
            .await
            .expect("writer-task receiver should yield the text frame");
        match msg {
            AxumMessage::Text(s) => assert_eq!(s, "hi"),
            other => panic!("expected Text(\"hi\"), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn send_json_serializes_value_and_dispatches_text_frame() {
        #[derive(serde::Serialize)]
        struct Greeting {
            hello: &'static str,
        }
        let (sender, mut rx) = WebSocketSender::channel();
        sender
            .send_json(&Greeting { hello: "world" })
            .expect("send_json on an open sender should succeed");
        let msg = rx
            .recv()
            .await
            .expect("writer-task receiver should yield the json frame");
        match msg {
            AxumMessage::Text(s) => assert_eq!(s, r#"{"hello":"world"}"#),
            other => panic!("expected Text(json), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn close_dispatches_empty_close_frame() {
        let (sender, mut rx) = WebSocketSender::channel();
        sender.close().expect("close on an open sender should succeed");
        let msg = rx
            .recv()
            .await
            .expect("writer-task receiver should yield the close frame");
        match msg {
            AxumMessage::Close(None) => {}
            other => panic!("expected Close(None), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn send_after_writer_receiver_drop_returns_closed_error() {
        // Models the connection-torn-down case: 124's writer task drops its
        // receiver when the axum sink errors or the close frame is observed.
        // Any subsequent send from a handler must surface as Closed, not
        // panic.
        let (sender, rx) = WebSocketSender::channel();
        drop(rx);
        let err = sender
            .send_text("hi")
            .expect_err("send on a dropped-receiver channel must fail");
        assert!(
            matches!(err, WsSendError::Closed),
            "expected WsSendError::Closed, got {err:?}"
        );
    }

    #[tokio::test]
    async fn cloning_a_sender_shares_the_same_writer_channel() {
        // Pins decision 5's contract: clones are cheap AND share the same
        // underlying writer-task feeder, so a handler that moves a clone
        // into a spawned task still writes to the same connection.
        let (sender, mut rx) = WebSocketSender::channel();
        let clone = sender.clone();
        sender.send_text("from-original").unwrap();
        clone.send_text("from-clone").unwrap();
        let first = rx.recv().await.expect("first frame");
        let second = rx.recv().await.expect("second frame");
        match (first, second) {
            (AxumMessage::Text(a), AxumMessage::Text(b)) => {
                assert_eq!(a, "from-original");
                assert_eq!(b, "from-clone");
            }
            other => panic!("expected two Text frames in order, got {other:?}"),
        }
    }

    #[test]
    fn axum_text_message_maps_to_ws_message_text() {
        // PRD-120 verification: "axum Text message → WsMessage::Text."
        let axum_msg = AxumMessage::Text("hello".to_string());
        let mapped: WsMessage = axum_msg.into();
        assert_eq!(mapped, WsMessage::Text("hello".to_string()));
    }

    #[test]
    fn axum_binary_ping_pong_messages_map_to_corresponding_ws_message_variants() {
        // Pins the lossless variant mapping for the three byte-payload
        // frame types. A regression that swapped Ping/Pong (an easy off-by-
        // one in the match arms) would fail two of these assertions.
        assert_eq!(
            WsMessage::from(AxumMessage::Binary(vec![1, 2, 3])),
            WsMessage::Binary(vec![1, 2, 3]),
        );
        assert_eq!(
            WsMessage::from(AxumMessage::Ping(vec![9])),
            WsMessage::Ping(vec![9]),
        );
        assert_eq!(
            WsMessage::from(AxumMessage::Pong(vec![9])),
            WsMessage::Pong(vec![9]),
        );
    }

    #[test]
    fn axum_close_none_and_close_some_map_with_owned_reason_string() {
        // Witness for the close-frame branch: None passes through, and the
        // axum Cow<'_, str> reason is materialized into an owned String so
        // WsMessage stays lifetime-free.
        assert_eq!(
            WsMessage::from(AxumMessage::Close(None)),
            WsMessage::Close(None),
        );

        let axum_frame = axum::extract::ws::CloseFrame {
            code: 1000,
            reason: std::borrow::Cow::Borrowed("normal"),
        };
        let mapped = WsMessage::from(AxumMessage::Close(Some(axum_frame)));
        assert_eq!(
            mapped,
            WsMessage::Close(Some(CloseFrame {
                code: 1000,
                reason: "normal".to_string(),
            })),
        );
    }

    #[test]
    fn ws_message_to_axum_message_round_trips_across_all_variants() {
        // The inverse mapping is what 124's writer task will exercise:
        // it converts outbound WsMessage frames back to AxumMessage. Going
        // WsMessage → AxumMessage → WsMessage should be the identity.
        let cases = vec![
            WsMessage::Text("hi".to_string()),
            WsMessage::Binary(vec![1, 2, 3]),
            WsMessage::Ping(vec![9]),
            WsMessage::Pong(vec![9]),
            WsMessage::Close(None),
            WsMessage::Close(Some(CloseFrame {
                code: 1011,
                reason: "server error".to_string(),
            })),
        ];
        for original in cases {
            let axum_form: AxumMessage = original.clone().into();
            let round_tripped: WsMessage = axum_form.into();
            assert_eq!(round_tripped, original);
        }
    }

    #[tokio::test]
    async fn send_dispatches_each_ws_message_variant_through_the_channel() {
        // Pins that WebSocketSender::send now actually dispatches (vs the
        // 119 `match msg {}` placeholder). One representative of each
        // variant goes in; the writer-task feeder receiver yields the
        // corresponding axum frame in order.
        let (sender, mut rx) = WebSocketSender::channel();
        sender.send(WsMessage::Text("t".to_string())).unwrap();
        sender.send(WsMessage::Binary(vec![1, 2])).unwrap();
        sender.send(WsMessage::Ping(vec![3])).unwrap();
        sender.send(WsMessage::Pong(vec![4])).unwrap();
        sender
            .send(WsMessage::Close(Some(CloseFrame {
                code: 1000,
                reason: "bye".to_string(),
            })))
            .unwrap();

        match rx.recv().await.unwrap() {
            AxumMessage::Text(s) => assert_eq!(s, "t"),
            other => panic!("expected Text, got {other:?}"),
        }
        match rx.recv().await.unwrap() {
            AxumMessage::Binary(b) => assert_eq!(b, vec![1, 2]),
            other => panic!("expected Binary, got {other:?}"),
        }
        match rx.recv().await.unwrap() {
            AxumMessage::Ping(p) => assert_eq!(p, vec![3]),
            other => panic!("expected Ping, got {other:?}"),
        }
        match rx.recv().await.unwrap() {
            AxumMessage::Pong(p) => assert_eq!(p, vec![4]),
            other => panic!("expected Pong, got {other:?}"),
        }
        match rx.recv().await.unwrap() {
            AxumMessage::Close(Some(frame)) => {
                assert_eq!(frame.code, 1000);
                assert_eq!(frame.reason, "bye");
            }
            other => panic!("expected Close(Some), got {other:?}"),
        }
    }

    #[test]
    fn ws_send_error_display_and_source_render_for_both_variants() {
        let closed = WsSendError::Closed;
        assert_eq!(closed.to_string(), "websocket sender channel is closed");
        assert!(std::error::Error::source(&closed).is_none());

        let serde_err = serde_json::from_str::<serde_json::Value>("not-json").unwrap_err();
        let wrapped = WsSendError::Serialize(serde_err);
        assert!(
            wrapped.to_string().starts_with("failed to serialize value to JSON:"),
            "got {wrapped}"
        );
        assert!(
            std::error::Error::source(&wrapped).is_some(),
            "Serialize variant must expose its serde_json source"
        );
    }
}
