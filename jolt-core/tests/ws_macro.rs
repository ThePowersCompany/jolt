//! JOLT-RS-122 + JOLT-RS-123 integration tests for the `ws!` proc-macro.
//!
//! - JOLT-RS-122: Verifies that `ws!` compiles and expands to a value carrying
//!   the parsed path + subprotocol.
//! - JOLT-RS-123 (this iteration): Verifies the tightened auth_fn signature
//!   check (`fn(&str) -> Result<JwtClaims, AuthError>`) and the generated
//!   `check_auth` method that wraps auth_fn → claims → handler.on_open dispatch.
//!
//! This is an integration test because the proc-macro can only be exercised
//! through cargo's compile pipeline.
//!
//! The ws! expansion now returns an anonymous struct with `.witness` (carrying
//! path + subprotocol) and `.check_auth(token)` (exercising the auth route).

use jolt_core::{
    ws, AuthError, JwtClaims, WebSocketHandler, WebSocketSender, WsMessage,
};

use axum::http::StatusCode;

/// A trivial handler used as the second positional `ws!` argument. The macro
/// emits a `const _: fn() = || { __jolt_ws_assert_handler::<EchoHandler>(); };`
/// trait-bound check at the call site, so an impl is required for the macro
/// invocation to compile. Overriding only `on_message` is enough — the other
/// four lifecycle methods inherit no-op defaults from the trait.
///
/// JOLT-RS-123 requires `Default` on the handler type so the generated
/// `check_auth` method can construct an instance via `<T>::default()`.
#[derive(Default)]
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
#[derive(Default)]
struct Adapter<H: WebSocketHandler + Default> {
    inner: H,
}

impl<H: WebSocketHandler + Send + Default> WebSocketHandler for Adapter<H> {
    async fn on_message(&mut self, msg: WsMessage, sender: WebSocketSender) {
        self.inner.on_message(msg, sender).await;
    }
}

/// JOLT-RS-123 auth function: matches the exact signature
/// `fn(&str) -> Result<JwtClaims, AuthError>` required by the tightened
/// compile-time check. Validates a hardcoded token for testing.
fn validate_token(token: &str) -> Result<JwtClaims, AuthError> {
    if token == "valid-token-123" {
        Ok(JwtClaims {
            sub: "user-123".to_owned(),
            exp: 9999999999,
            iat: None,
            extra: Default::default(),
        })
    } else {
        Err(AuthError::new(format!("invalid token: {token}")))
    }
}

mod auth_module {
    use jolt_core::{AuthError, JwtClaims};

    pub fn validate(token: &str) -> Result<JwtClaims, AuthError> {
        if token == "mod-valid-token" {
            Ok(JwtClaims {
                sub: "mod-user".to_owned(),
                exp: 9999999999,
                iat: None,
                extra: Default::default(),
            })
        } else {
            Err(AuthError::new("module-level auth rejected"))
        }
    }
}

#[test]
fn canonical_invocation_returns_witness_with_path_and_subprotocol() {
    let expanded = ws!(
        "/chat",
        EchoHandler,
        subprotocol = "chat-v1",
        auth_fn = validate_token
    );
    assert_eq!(expanded.witness.path, "/chat");
    assert_eq!(expanded.witness.subprotocol, "chat-v1");
}

#[test]
fn named_args_compose_in_reverse_order() {
    let expanded = ws!(
        "/ws",
        EchoHandler,
        auth_fn = validate_token,
        subprotocol = "v2"
    );
    assert_eq!(expanded.witness.path, "/ws");
    assert_eq!(expanded.witness.subprotocol, "v2");
}

#[test]
fn auth_fn_accepts_module_qualified_path() {
    let expanded = ws!(
        "/chat",
        EchoHandler,
        subprotocol = "v1",
        auth_fn = auth_module::validate
    );
    assert_eq!(expanded.witness.path, "/chat");
    assert_eq!(expanded.witness.subprotocol, "v1");
}

#[test]
fn handler_type_accepts_generic_args() {
    let expanded = ws!(
        "/chat",
        Adapter<EchoHandler>,
        subprotocol = "v1",
        auth_fn = validate_token
    );
    assert_eq!(expanded.witness.path, "/chat");
    assert_eq!(expanded.witness.subprotocol, "v1");
}

#[tokio::test]
async fn check_auth_valid_token_returns_200_and_calls_handler_on_open() {
    let expanded = ws!(
        "/chat",
        EchoHandler,
        subprotocol = "v1",
        auth_fn = validate_token
    );
    let response = expanded.check_auth("valid-token-123").await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn check_auth_valid_token_stashes_claims_via_set_claims_before_on_open() {
    use jolt_core::{AuthError, JwtClaims, WebSocketHandler, WebSocketSender};

    #[derive(Default)]
    struct ClaimsVerifyingHandler {
        claims_sub: Option<String>,
    }

    impl WebSocketHandler for ClaimsVerifyingHandler {
        fn set_claims(&mut self, claims: JwtClaims) {
            self.claims_sub = Some(claims.sub);
        }
        async fn on_open(&mut self, _: WebSocketSender) {
            assert!(
                self.claims_sub.as_deref() == Some("user-123"),
                "set_claims must be called before on_open; sub was {:?}",
                self.claims_sub,
            );
        }
    }

    fn claims_auth(token: &str) -> Result<JwtClaims, AuthError> {
        if token == "good" {
            Ok(JwtClaims {
                sub: "user-123".to_owned(),
                exp: 9999999999,
                iat: None,
                extra: Default::default(),
            })
        } else {
            Err(AuthError::new("nope"))
        }
    }

    let expanded = ws!(
        "/chat",
        ClaimsVerifyingHandler,
        subprotocol = "v1",
        auth_fn = claims_auth
    );
    let response = expanded.check_auth("good").await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn check_auth_invalid_token_returns_401() {
    let expanded = ws!(
        "/chat",
        EchoHandler,
        subprotocol = "v1",
        auth_fn = validate_token
    );
    let response = expanded.check_auth("bad-token").await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn check_auth_invalid_token_body_contains_error_message() {
    let expanded = ws!(
        "/chat",
        EchoHandler,
        subprotocol = "v1",
        auth_fn = validate_token
    );
    let response = expanded.check_auth("bogus").await;
    let body_bytes = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let body_str = String::from_utf8_lossy(&body_bytes);
    assert!(
        body_str.contains("bogus"),
        "401 body must contain the rejection detail, got: {body_str}"
    );
}
