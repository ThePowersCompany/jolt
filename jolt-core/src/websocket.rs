//! WebSocket lifecycle abstractions (phase28-ws-trait).
//!
//! Phase28 ladder:
//! - JOLT-RS-118 (this iteration): define the [`WebSocketHandler`] trait with
//!   five async lifecycle methods (`on_open`, `on_ready`, `on_message`,
//!   `on_close`, `on_shutdown`) and no-op default impls. Define minimal
//!   placeholder shapes for [`WebSocketSender`] and [`WsMessage`] so the
//!   trait signatures can reference them today; 119 fleshes out
//!   `WebSocketSender` (wraps axum's sender half + send/send_text/send_json/
//!   close), and 120 fleshes out `WsMessage` (Text/Binary/Ping/Pong/Close
//!   variants + axum `Message` ↔ `WsMessage` mapping).
//! - JOLT-RS-121: write the lifecycle-callback ordering unit test
//!   (open → ready → message → close fires in order on a mock impl).
//! - JOLT-RS-122..125: the `ws!` macro that registers a handler against an
//!   axum router behind a JWT-subprotocol auth guard (consumes the trait
//!   declared here, composes with [`AuthWsJwtLayer`](crate::AuthWsJwtLayer)
//!   from JOLT-RS-076).
//!
//! Architectural decisions pinned here for 119..125 to build on:
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
//! 3. **`WebSocketSender` and `WsMessage` are declared as opaque placeholders
//!    at 118 and fleshed out at 119 / 120.** The trait's method signatures
//!    need the types to exist *as names*, but their internal shape belongs
//!    to the follow-up slices. The placeholders are intentionally
//!    constructible inside the defining crate (a `#[non_exhaustive]` unit
//!    struct and a `#[non_exhaustive]` empty enum) so:
//!     - `WebSocketSender` resolves to a type today that downstream crates
//!       cannot construct (forcing them to go through 119's
//!       `WebSocketSender::new(_)` constructor once it lands), but jolt-core's
//!       own 118 unit test can pass to the trait's default `on_open`.
//!     - `WsMessage` is an uninhabited enum at 118 (no variants); 120 will
//!       add variants additively. No-op `on_message` default impl never
//!       matches on it; 121's mock-handler test (which constructs
//!       variants) lands after 120.
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
//!    methods, and `on_close` / `on_shutdown` take no sender.** 119 will
//!    make `WebSocketSender` cheaply clonable (it wraps an `Arc`-backed
//!    handle to the writer task), so passing by value is no more expensive
//!    than passing `&WebSocketSender` and lets handlers move the sender into
//!    spawned tasks (`tokio::spawn`-and-forget for pub/sub fan-in,
//!    heartbeats, etc.) without lifetime gymnastics. `on_close` and
//!    `on_shutdown` take no sender — the socket is being torn down; sending
//!    into it is a no-op at best and a panic at worst.
//!
//! 6. **No trait-level `Error` associated type.** The PRD-118 signatures are
//!    all `async fn ... -> ()`; lifecycle errors are the handler's
//!    responsibility to log / surface internally. 119's
//!    `WebSocketSender::send_*` returns `Result<(), _>` (errors from the
//!    underlying axum sink), but those errors don't propagate up the trait —
//!    the handler decides how to react (drop the connection, retry, log).
//!    Adding a trait-level `Error` associated type would force every handler
//!    to declare one, even for the common case where the handler swallows
//!    all errors internally.

/// Server-side half of a WebSocket connection. Passed by value into
/// [`WebSocketHandler::on_open`], [`WebSocketHandler::on_ready`], and
/// [`WebSocketHandler::on_message`].
///
/// At JOLT-RS-118 this is a `#[non_exhaustive]` unit-struct placeholder so
/// the [`WebSocketHandler`] trait's signatures can name it. JOLT-RS-119
/// replaces the body with an axum-sender wrapper and adds the
/// `send` / `send_text` / `send_json` / `close` methods; 119's additive
/// changes won't break any 118-era code because `#[non_exhaustive]` already
/// forbids downstream construction with field/struct syntax.
#[non_exhaustive]
pub struct WebSocketSender;

/// Inbound WebSocket frame delivered to [`WebSocketHandler::on_message`].
///
/// At JOLT-RS-118 this is a `#[non_exhaustive]` empty enum so the
/// [`WebSocketHandler`] trait's signatures can name it. The type is
/// uninhabited today — no variants can be constructed and no `match` arm
/// can fire — which is sound for 118's default-impl-only test. JOLT-RS-120
/// adds the `Text(String)`, `Binary(Vec<u8>)`, `Ping(Vec<u8>)`,
/// `Pong(Vec<u8>)`, and `Close(Option<CloseFrame>)` variants and the
/// axum `Message` ↔ `WsMessage` mapping; downstream `match` expressions on
/// `WsMessage` must include a `_ =>` wildcard until then (enforced by
/// `#[non_exhaustive]`).
#[non_exhaustive]
pub enum WsMessage {}

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
///     async fn on_message(&mut self, _msg: WsMessage, _sender: WebSocketSender) {
///         // 120 adds WsMessage variants; 119 adds WebSocketSender::send_text.
///         // At 118 this body is unreachable — WsMessage is uninhabited.
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

#[cfg(test)]
mod tests {
    //! PRD-mandated verification for JOLT-RS-118: "Trait compiles and can be
    //! implemented." The tests below pin three claims about the trait:
    //!
    //! 1. A unit struct can `impl WebSocketHandler for ...` with zero method
    //!    overrides (`no_override_compiles_with_defaults`).
    //! 2. A handler that overrides some methods still satisfies the trait
    //!    bound (`partial_override_satisfies_trait_bound`).
    //! 3. The default impls actually execute without panicking
    //!    (`default_impls_run_to_completion_on_unit_struct_handler`). The
    //!    `on_message` default IS exercised — but the only way to construct
    //!    a `WsMessage` today is via an uninhabited `match never {}` shim,
    //!    so 118 covers only the four non-`on_message` defaults; 121 will
    //!    add the `on_message` ordering coverage once 120 lands the
    //!    variants.
    //!
    //! These are compile-time-plus-runtime witnesses rather than a single
    //! marker const (the parse-witness shape used by JOLT-RS-046 / 110)
    //! because a trait declaration has no parse-output artifact to assert
    //! against — the trait itself IS the artifact, so the tests prove the
    //! shape by using it.

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
            // Body never executes today (WsMessage is uninhabited at 118);
            // 121 will reach this branch after 120 adds variants.
            unreachable!("WsMessage has no variants at JOLT-RS-118");
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
        // 118; 121 covers it once 120 lands the variants.
        let mut handler = NoOverrideHandler;
        handler.on_open(WebSocketSender).await;
        handler.on_ready(WebSocketSender).await;
        handler.on_close().await;
        handler.on_shutdown().await;
    }

    #[tokio::test]
    async fn partial_override_fires_overridden_on_open_and_leaves_others_default() {
        // Confirms that overriding on_open doesn't accidentally shadow the
        // sibling defaults — on_close and on_shutdown still run as no-ops.
        let mut handler = PartialOverrideHandler { open_called: false };
        handler.on_open(WebSocketSender).await;
        assert!(handler.open_called, "on_open override must have fired");
        handler.on_ready(WebSocketSender).await;
        handler.on_close().await;
        handler.on_shutdown().await;
    }
}
