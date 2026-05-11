//! JOLT-RS-122 PRD-mandated integration test.
//!
//! Verifies that the `ws!` function-like proc-macro compiles and expands to
//! valid code — the PRD's verification line: "Macro compiles and expands to
//! valid code."
//!
//! This is an integration test (not a unit test) because the proc-macro can
//! only be exercised through cargo's compile pipeline. The macro crate's own
//! unit tests parse-check the emitted token stream but cannot expand and
//! type-check the macro against real call sites in a downstream crate.
//!
//! The hidden `__WsMacroWitness` struct returned by the expansion is the
//! observable witness that parsing + expansion succeeded. Later phase29 items
//! (123-125) replace the witness return with the real axum WS upgrade
//! handler; the witness stays alongside the new surface until 125's
//! integration test (full connect / send / receive) makes it redundant.

use jolt_core::{ws, WebSocketHandler, WebSocketSender, WsMessage, __WsMacroWitness};

/// A trivial handler used as the second positional `ws!` argument. The macro
/// emits a `const _: fn() = || { __jolt_ws_assert_handler::<EchoHandler>(); };`
/// trait-bound check at the call site, so an impl is required for the macro
/// invocation to compile. Overriding only `on_message` is enough — the other
/// four lifecycle methods inherit no-op defaults from the trait.
struct EchoHandler;

impl WebSocketHandler for EchoHandler {
    async fn on_message(&mut self, msg: WsMessage, sender: WebSocketSender) {
        if let WsMessage::Text(text) = msg {
            let _ = sender.send_text(&text);
        }
    }
}

/// A second handler with generic args, used to pin that the macro accepts a
/// complex `syn::Type` for the handler argument (not just a bare ident). A
/// regression that hard-coded `Ident` parsing would surface as a compile
/// error on the `ws!(... Adapter<EchoHandler> ...)` invocation below.
struct Adapter<H: WebSocketHandler> {
    inner: H,
}

impl<H: WebSocketHandler + Send> WebSocketHandler for Adapter<H> {
    async fn on_message(&mut self, msg: WsMessage, sender: WebSocketSender) {
        self.inner.on_message(msg, sender).await;
    }
}

/// Stub auth function matching the future `fn(&str) -> Result<JwtClaims, _>`
/// signature 123 will pin. At 122 the macro type-checks only that the path
/// resolves to a value, so any callable shape compiles.
fn validate_token(_token: &str) -> Result<(), ()> {
    Ok(())
}

mod auth_module {
    pub fn validate(_token: &str) -> Result<(), ()> {
        Ok(())
    }
}

#[test]
fn canonical_invocation_returns_witness_with_path_and_subprotocol() {
    // The PRD verification target: the canonical invocation form must
    // compile and expand to a value carrying the parsed path + subprotocol.
    let witness: __WsMacroWitness = ws!(
        "/chat",
        EchoHandler,
        subprotocol = "chat-v1",
        auth_fn = validate_token
    );
    assert_eq!(witness.path, "/chat");
    assert_eq!(witness.subprotocol, "chat-v1");
}

#[test]
fn named_args_compose_in_reverse_order() {
    // Named args may appear in either order after the two positional args.
    // A regression that hard-coded the `subprotocol` then `auth_fn` order
    // would surface as a "missing required argument" diagnostic at the
    // call site of this test.
    let witness: __WsMacroWitness = ws!(
        "/ws",
        EchoHandler,
        auth_fn = validate_token,
        subprotocol = "v2"
    );
    assert_eq!(witness.path, "/ws");
    assert_eq!(witness.subprotocol, "v2");
}

#[test]
fn auth_fn_accepts_module_qualified_path() {
    // `auth_fn` is a `syn::Path`, not just an ident — a module-qualified
    // function must work. Pinning this protects against a regression that
    // parsed an `Ident` for the auth_fn argument.
    let witness: __WsMacroWitness = ws!(
        "/chat",
        EchoHandler,
        subprotocol = "v1",
        auth_fn = auth_module::validate
    );
    assert_eq!(witness.path, "/chat");
    assert_eq!(witness.subprotocol, "v1");
}

#[test]
fn handler_type_accepts_generic_args() {
    // The handler argument is a full `syn::Type`. Pinning a generic-args
    // adapter pattern here protects against a regression that parsed only
    // a bare type ident.
    let witness: __WsMacroWitness = ws!(
        "/chat",
        Adapter<EchoHandler>,
        subprotocol = "v1",
        auth_fn = validate_token
    );
    assert_eq!(witness.path, "/chat");
    assert_eq!(witness.subprotocol, "v1");
}
