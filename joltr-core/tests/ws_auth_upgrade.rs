use std::io::ErrorKind;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::ws::{WebSocket, WebSocketUpgrade};
use axum::routing::get;
use axum::Extension;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use joltr_core::{AuthWsJwtLayer, WebSocketHandler};
use joltr_utils::jwt::{JwtClaims, JwtConfig};

#[tokio::test]
async fn ws_connect_with_valid_token_upgrade_succeeds_and_handler_has_claims() {
    let _ = tracing_subscriber::fmt().try_init();

    let secret = b"joltr-077-test-secret-key";
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let expected_sub = "user-077".to_owned();
    let claims = JwtClaims {
        sub: Some(expected_sub.clone()),
        exp: Some(now + 3600),
        iat: Some(now),
        nbf: None,
        iss: None,
        aud: None,
        custom: Default::default(),
    };
    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret),
    )
    .expect("HS256 encoding of valid claims must succeed");

    let config = JwtConfig::new(secret.to_vec(), Algorithm::HS256);

    let received_claims: Arc<Mutex<Option<JwtClaims>>> = Arc::new(Mutex::new(None));
    let state = received_claims.clone();

    let app = axum::Router::new()
        .route(
            "/ws",
            get(
                |ws: WebSocketUpgrade,
                 Extension(extracted_claims): Extension<JwtClaims>| async move {
                    let state = state.clone();
                    ws.on_upgrade(move |_socket: WebSocket| async move {
                        let mut handler = ClaimsCapturingHandler {
                            received: state,
                        };
                        handler.set_claims(extracted_claims);
                    })
                },
            ),
        )
        .layer(AuthWsJwtLayer::new(config));

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
         Sec-WebSocket-Protocol: joltr-jwt, {token}\r\n\
         \r\n"
    );
    stream.write_all(upgrade_request.as_bytes()).await.unwrap();

    let mut buf = vec![0u8; 4096];
    let n = read_response(&mut stream, &mut buf).await;
    let response = String::from_utf8_lossy(&buf[..n]);

    assert!(
        response.starts_with("HTTP/1.1 101"),
        "expected 101 Switching Protocols, got: {response}"
    );
    assert!(
        response.to_lowercase().contains("upgrade: websocket"),
        "response must confirm websocket upgrade, got: {response}"
    );

    tokio::time::sleep(Duration::from_millis(100)).await;
    let stored = received_claims.lock().unwrap();
    let stored = stored
        .as_ref()
        .expect("handler.set_claims must have been called");
    assert_eq!(stored.sub.as_deref(), Some(expected_sub.as_str()));
    assert_eq!(stored.exp, claims.exp);
    assert!(
        stored.iat.is_some(),
        "iat claim must be preserved through the auth pipeline"
    );
}

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

struct ClaimsCapturingHandler {
    received: Arc<Mutex<Option<JwtClaims>>>,
}

impl WebSocketHandler for ClaimsCapturingHandler {
    fn set_claims(&mut self, claims: JwtClaims) {
        *self.received.lock().unwrap() = Some(claims);
    }
}

#[tokio::test]
async fn ws_connect_with_invalid_token_is_rejected_with_401() {
    let _ = tracing_subscriber::fmt().try_init();

    let secret = b"joltr-077-test-secret-key";
    let config = JwtConfig::new(secret.to_vec(), Algorithm::HS256);

    let app = axum::Router::new()
        .route(
            "/ws",
            get(|ws: WebSocketUpgrade| async move {
                ws.on_upgrade(|_socket: WebSocket| async move {})
            }),
        )
        .layer(AuthWsJwtLayer::new(config));

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
         Sec-WebSocket-Protocol: joltr-jwt, definitely-not-a-valid-jwt\r\n\
         \r\n"
    );
    stream.write_all(upgrade_request.as_bytes()).await.unwrap();

    let mut buf = vec![0u8; 4096];
    let n = read_response(&mut stream, &mut buf).await;
    let response = String::from_utf8_lossy(&buf[..n]);

    assert!(
        response.starts_with("HTTP/1.1 401"),
        "expected 401 Unauthorized for invalid token, got: {response}"
    );
    assert!(
        !response.contains("HTTP/1.1 101"),
        "must NOT upgrade for invalid token, got: {response}"
    );
}
