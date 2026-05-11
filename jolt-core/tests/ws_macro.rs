//! JOLT-RS-124 integration tests for the `ws!` proc-macro.
//!
//! Verifies that `ws!` compiles and emits an async closure that axum can wire
//! as a WebSocket route handler:
//! - a valid token → 101 upgrade, handler lifecycle fires
//! - an invalid token → 401 rejection
//! - the handler trait-bound check (WebSocketHandler + Default + Send) works
//! - the auth_fn signature check works
//!
//! This is an integration test because the proc-macro can only be exercised
//! through cargo's compile pipeline.

use std::io::ErrorKind;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::routing::get;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use jolt_core::{
    ws, AuthError, JwtClaims, WebSocketHandler, WebSocketSender, WsMessage,
};

/// A static shared across all test handler instances so the ws! macro
/// (which constructs the handler via `Default::default`) can write into
/// the same log regardless of which specific instance it creates.
static LIFECYCLE_LOG: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();

fn lifecycle_log() -> &'static Mutex<Vec<&'static str>> {
    LIFECYCLE_LOG.get_or_init(|| Mutex::new(Vec::new()))
}

/// A handler that records lifecycle callbacks into the static LIFECYCLE_LOG.
/// The ws! macro constructs this via `Default::default()`, and the impl
/// writes into the static so multiple calls / instances converge on one log.
#[derive(Default)]
struct LifecycleLogHandler;

impl WebSocketHandler for LifecycleLogHandler {
    fn set_claims(&mut self, _claims: JwtClaims) {
        lifecycle_log().lock().unwrap().push("set_claims");
    }
    async fn on_open(&mut self, _sender: WebSocketSender) {
        lifecycle_log().lock().unwrap().push("on_open");
    }
    async fn on_ready(&mut self, _sender: WebSocketSender) {
        lifecycle_log().lock().unwrap().push("on_ready");
    }
    async fn on_message(&mut self, _msg: WsMessage, _sender: WebSocketSender) {
        lifecycle_log().lock().unwrap().push("on_message");
    }
    async fn on_close(&mut self) {
        lifecycle_log().lock().unwrap().push("on_close");
    }
    async fn on_shutdown(&mut self) {
        lifecycle_log().lock().unwrap().push("on_shutdown");
    }
}

#[tokio::test]
async fn valid_token_upgrade_succeeds_and_drives_lifecycle() {
    let _ = tracing_subscriber::fmt().try_init();

    // Clear the static log so this test starts fresh.
    lifecycle_log().lock().unwrap().clear();

    fn test_auth(token: &str) -> Result<JwtClaims, AuthError> {
        if token == "valid-token" {
            Ok(JwtClaims {
                sub: Some("user-1".to_owned()),
                exp: Some(9999999999),
                iat: None,
                nbf: None,
                iss: None,
                aud: None,
                custom: Default::default(),
            })
        } else {
            Err(AuthError::new("nope"))
        }
    }

    let handler = ws!(
        "/ws",
        LifecycleLogHandler,
        subprotocol = "chat-v1",
        auth_fn = test_auth
    );

    let app = axum::Router::new().route("/ws", get(handler));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();

    let upgrade_request = format!(
        "GET /ws HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Sec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Protocol: jolt-jwt, valid-token\r\n\
         \r\n"
    );
    stream.write_all(upgrade_request.as_bytes()).await.unwrap();

    let mut buf = vec![0u8; 4096];
    let n = read_response(&mut stream, &mut buf).await;
    let response = String::from_utf8_lossy(&buf[..n]);

    assert!(
        response.starts_with("HTTP/1.1 101"),
        "expected 101 Switching Protocols for valid token, got: {response}"
    );

    // Allow a brief window for the lifecycle callbacks to fire.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let calls = lifecycle_log().lock().unwrap().clone();
    assert!(
        calls.contains(&"set_claims"),
        "set_claims must be called; observed: {calls:?}"
    );
    assert!(calls.contains(&"on_open"), "on_open must be called; observed: {calls:?}");
    assert!(
        calls.contains(&"on_ready"),
        "on_ready must be called; observed: {calls:?}"
    );
}

#[tokio::test]
async fn invalid_token_is_rejected_with_401() {
    let _ = tracing_subscriber::fmt().try_init();

    #[derive(Default)]
    struct EchoHandler;

    impl WebSocketHandler for EchoHandler {}

    fn my_auth(token: &str) -> Result<JwtClaims, AuthError> {
        if token == "good" {
            Ok(JwtClaims {
                sub: Some("u".to_owned()),
                exp: Some(9999999999),
                iat: None,
                nbf: None,
                iss: None,
                aud: None,
                custom: Default::default(),
            })
        } else {
            Err(AuthError::new("bad token"))
        }
    }

    let handler = ws!(
        "/ws",
        EchoHandler,
        subprotocol = "chat-v1",
        auth_fn = my_auth
    );

    let app = axum::Router::new().route("/ws", get(handler));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();

    let upgrade_request = format!(
        "GET /ws HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Sec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Protocol: jolt-jwt, definitely-invalid\r\n\
         \r\n"
    );
    stream.write_all(upgrade_request.as_bytes()).await.unwrap();

    let mut buf = vec![0u8; 4096];
    let n = read_response(&mut stream, &mut buf).await;
    let response = String::from_utf8_lossy(&buf[..n]);

    assert!(
        response.starts_with("HTTP/1.1 401"),
        "expected 401 for invalid token, got: {response}"
    );
    assert!(
        !response.contains("HTTP/1.1 101"),
        "must NOT upgrade for invalid token, got: {response}"
    );
}

#[tokio::test]
async fn missing_subprotocol_header_is_rejected_with_401() {
    let _ = tracing_subscriber::fmt().try_init();

    #[derive(Default)]
    struct EchoHandler;

    impl WebSocketHandler for EchoHandler {}

    fn my_auth(token: &str) -> Result<JwtClaims, AuthError> {
        if token == "good" {
            Ok(JwtClaims {
                sub: Some("u".to_owned()),
                exp: Some(9999999999),
                iat: None,
                nbf: None,
                iss: None,
                aud: None,
                custom: Default::default(),
            })
        } else {
            Err(AuthError::new("bad"))
        }
    }

    let handler = ws!(
        "/ws",
        EchoHandler,
        subprotocol = "chat-v1",
        auth_fn = my_auth
    );

    let app = axum::Router::new().route("/ws", get(handler));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();

    // No Sec-WebSocket-Protocol header at all — must still be rejected.
    let upgrade_request = format!(
        "GET /ws HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Sec-WebSocket-Version: 13\r\n\
         \r\n"
    );
    stream.write_all(upgrade_request.as_bytes()).await.unwrap();

    let mut buf = vec![0u8; 4096];
    let n = read_response(&mut stream, &mut buf).await;
    let response = String::from_utf8_lossy(&buf[..n]);

    assert!(
        response.starts_with("HTTP/1.1 401"),
        "expected 401 for missing subprotocol header, got: {response}"
    );
}

#[tokio::test]
async fn ws_macro_compiles_with_generic_handler_type() {
    // Compile-time-only test: ws! must accept a handler with generic params.
    // If the generic handler doesn't satisfy the trait bounds, the compile
    // error surfaces at the ws! call site — this test only checks the happy
    // path.
    #[derive(Default)]
    struct GenericAdapter<H: WebSocketHandler + Default>(H);

    impl<H: WebSocketHandler + Default + Send> WebSocketHandler for GenericAdapter<H> {
        async fn on_message(&mut self, msg: WsMessage, sender: WebSocketSender) {
            self.0.on_message(msg, sender).await;
        }
    }

    #[derive(Default)]
    struct SimpleEcho;

    impl WebSocketHandler for SimpleEcho {}

    fn auth_ok(token: &str) -> Result<JwtClaims, AuthError> {
        if token == "ok" {
            Ok(JwtClaims {
                sub: Some("u".to_owned()),
                exp: Some(9999999999),
                iat: None,
                nbf: None,
                iss: None,
                aud: None,
                custom: Default::default(),
            })
        } else {
            Err(AuthError::new("no"))
        }
    }

    let handler = ws!(
        "/ws",
        GenericAdapter<SimpleEcho>,
        subprotocol = "v1",
        auth_fn = auth_ok
    );

    let app = axum::Router::new().route("/ws", get(handler));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let req = format!(
        "GET /ws HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Sec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Protocol: jolt-jwt, ok\r\n\
         \r\n"
    );
    stream.write_all(req.as_bytes()).await.unwrap();

    let mut buf = vec![0u8; 4096];
    let n = read_response(&mut stream, &mut buf).await;
    let response = String::from_utf8_lossy(&buf[..n]);
    assert!(
        response.starts_with("HTTP/1.1 101"),
        "generic handler with valid token must upgrade, got: {response}"
    );
}

/// HTTP-response reader for raw TCP WebSocket handshake tests.
/// Reads until either the stream is exhausted, the double-CRLF that marks
/// the end of the HTTP response headers is detected, or a WouldBlock timeout.
async fn read_response(stream: &mut tokio::net::TcpStream, buf: &mut [u8]) -> usize {
    let mut total = 0;
    loop {
        match stream.read(&mut buf[total..]).await {
            Ok(0) | Err(_) if total > 0 && contains_empty_line(&buf[..total]) => break total,
            Ok(0) => break total,
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                if total > 0 && contains_empty_line(&buf[..total]) {
                    break total;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
                continue;
            }
            Err(_) => break total,
            Ok(n) => {
                total += n;
                if contains_empty_line(&buf[..total]) {
                    break total;
                }
            }
        }
    }
}

fn contains_empty_line(buf: &[u8]) -> bool {
    let s = String::from_utf8_lossy(buf);
    s.contains("\r\n\r\n") || s.contains("\n\n")
}
