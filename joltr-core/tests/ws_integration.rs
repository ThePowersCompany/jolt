use std::io::ErrorKind;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::routing::get;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use joltr_core::{ws, AuthError, JwtClaims, WebSocketHandler, WebSocketSender, WsMessage};

static RECEIVED: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
static LIFECYCLE: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
static WS_TEST_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

fn received() -> &'static Mutex<Vec<String>> {
    RECEIVED.get_or_init(|| Mutex::new(Vec::new()))
}

fn lifecycle() -> &'static Mutex<Vec<&'static str>> {
    LIFECYCLE.get_or_init(|| Mutex::new(Vec::new()))
}

fn ws_test_lock() -> &'static tokio::sync::Mutex<()> {
    WS_TEST_LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

#[derive(Default)]
struct EchoHandler;

impl WebSocketHandler for EchoHandler {
    fn set_claims(&mut self, _claims: JwtClaims) {
        lifecycle().lock().unwrap().push("set_claims");
    }

    async fn on_open(&mut self, sender: WebSocketSender) {
        lifecycle().lock().unwrap().push("on_open");
        let _ = sender.send_text("welcome");
    }

    async fn on_ready(&mut self, _sender: WebSocketSender) {
        lifecycle().lock().unwrap().push("on_ready");
    }

    async fn on_message(&mut self, msg: WsMessage, sender: WebSocketSender) {
        match &msg {
            WsMessage::Text(text) => {
                lifecycle().lock().unwrap().push("on_message:text");
                received().lock().unwrap().push(text.clone());
                let _ = sender.send_text(text);
                let _ = sender.close();
            }
            WsMessage::Close(_) => {
                lifecycle().lock().unwrap().push("on_message:close");
            }
            _ => {}
        }
    }

    async fn on_close(&mut self) {
        lifecycle().lock().unwrap().push("on_close");
    }

    async fn on_shutdown(&mut self) {
        lifecycle().lock().unwrap().push("on_shutdown");
    }
}

fn auth_ok(token: &str) -> Result<JwtClaims, AuthError> {
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
        Err(AuthError::new("invalid token"))
    }
}

#[tokio::test]
async fn valid_token_upgrade_and_message_exchange() {
    let _guard = ws_test_lock().lock().await;
    let _ = tracing_subscriber::fmt().try_init();
    received().lock().unwrap().clear();
    lifecycle().lock().unwrap().clear();

    let handler = ws!(
        "/ws",
        EchoHandler,
        subprotocol = "chat-v1",
        auth_fn = auth_ok
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
         Sec-WebSocket-Protocol: joltr-jwt, valid-token\r\n\
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

    let first_frame = recv_frame(&mut stream).await;
    assert_eq!(
        first_frame, "welcome",
        "expected 'welcome' from on_open, got: {first_frame:?}"
    );

    send_text_frame(&mut stream, "hello").await;

    let echo_frame = recv_frame(&mut stream).await;
    assert_eq!(
        echo_frame, "hello",
        "expected echo of 'hello', got: {echo_frame:?}"
    );

    assert!(
        received().lock().unwrap().contains(&"hello".to_owned()),
        "handler must have received the message"
    );

    let close_frame = recv_frame_raw(&mut stream).await;
    assert_eq!(
        close_frame,
        Some(vec![0x88, 0x00]),
        "expected queued close frame from on_message before shutdown"
    );

    send_close_frame(&mut stream).await;

    let calls = wait_for_lifecycle_shutdown().await;
    assert_eq!(
        calls,
        vec![
            "set_claims",
            "on_open",
            "on_ready",
            "on_message:text",
            "on_message:close",
            "on_close",
            "on_shutdown",
        ],
        "websocket lifecycle callbacks must run in documented order"
    );
}

#[tokio::test]
async fn invalid_token_is_rejected_with_401() {
    let _guard = ws_test_lock().lock().await;
    let _ = tracing_subscriber::fmt().try_init();

    #[derive(Default)]
    struct NoopHandler;

    impl WebSocketHandler for NoopHandler {}

    fn strict_auth(token: &str) -> Result<JwtClaims, AuthError> {
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
        NoopHandler,
        subprotocol = "chat-v1",
        auth_fn = strict_auth
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
         Sec-WebSocket-Protocol: joltr-jwt, bad-token\r\n\
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

async fn send_text_frame(stream: &mut tokio::net::TcpStream, payload: &str) {
    let data = payload.as_bytes();
    let frame = build_masked_frame(0x1, data); // text opcode
    stream.write_all(&frame).await.unwrap();
}

async fn send_close_frame(stream: &mut tokio::net::TcpStream) {
    let frame = build_masked_frame(0x8, &[]); // close opcode, no payload
    stream.write_all(&frame).await.unwrap();
}

fn build_masked_frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let len = payload.len();
    let mut frame = Vec::new();

    frame.push(0x80 | opcode); // FIN | opcode

    if len < 126 {
        frame.push(0x80 | len as u8); // MASK | len
    } else if len <= 0xFFFF {
        frame.push(0x80 | 126); // MASK | 126
        frame.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        frame.push(0x80 | 127); // MASK | 127
        frame.extend_from_slice(&(len as u64).to_be_bytes());
    }

    let mask_key: [u8; 4] = rand_mask();
    frame.extend_from_slice(&mask_key);

    let masked: Vec<u8> = payload
        .iter()
        .enumerate()
        .map(|(i, b)| b ^ mask_key[i % 4])
        .collect();
    frame.extend_from_slice(&masked);

    frame
}

fn rand_mask() -> [u8; 4] {
    use std::time::SystemTime;
    let t = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    [(t >> 24) as u8, (t >> 16) as u8, (t >> 8) as u8, t as u8]
}

async fn recv_frame(stream: &mut tokio::net::TcpStream) -> String {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);

    loop {
        let Some((opcode, payload, _raw)) = recv_frame_parts(stream, deadline).await else {
            break;
        };

        match opcode {
            0x1 => return String::from_utf8_lossy(&payload).to_string(),
            0x9 => send_pong(stream, &payload).await,
            0x8 => break,
            _ => {}
        }
    }

    String::new()
}

async fn recv_frame_raw(stream: &mut tokio::net::TcpStream) -> Option<Vec<u8>> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let (opcode, _payload, raw) = recv_frame_parts(stream, deadline).await?;
        if opcode == 0x8 {
            return Some(raw);
        }
    }
}

async fn recv_frame_parts(
    stream: &mut tokio::net::TcpStream,
    deadline: tokio::time::Instant,
) -> Option<(u8, Vec<u8>, Vec<u8>)> {
    let mut header = [0u8; 2];
    read_exact_before(stream, &mut header, deadline).await?;

    let opcode = header[0] & 0x0F;
    let masked = (header[1] & 0x80) != 0;
    let mut payload_len = (header[1] & 0x7F) as usize;
    let mut raw = header.to_vec();

    if payload_len == 126 {
        let mut extended = [0u8; 2];
        read_exact_before(stream, &mut extended, deadline).await?;
        raw.extend_from_slice(&extended);
        payload_len = u16::from_be_bytes(extended) as usize;
    } else if payload_len == 127 {
        let mut extended = [0u8; 8];
        read_exact_before(stream, &mut extended, deadline).await?;
        raw.extend_from_slice(&extended);
        payload_len = u64::from_be_bytes(extended) as usize;
    }

    let mask_key = if masked {
        let mut key = [0u8; 4];
        read_exact_before(stream, &mut key, deadline).await?;
        raw.extend_from_slice(&key);
        Some(key)
    } else {
        None
    };

    let mut payload = vec![0u8; payload_len];
    read_exact_before(stream, &mut payload, deadline).await?;
    raw.extend_from_slice(&payload);

    if let Some(mask_key) = mask_key {
        for (index, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask_key[index % 4];
        }
    }

    Some((opcode, payload, raw))
}

async fn read_exact_before(
    stream: &mut tokio::net::TcpStream,
    buf: &mut [u8],
    deadline: tokio::time::Instant,
) -> Option<()> {
    let remaining = deadline.checked_duration_since(tokio::time::Instant::now())?;
    match tokio::time::timeout(remaining, stream.read_exact(buf)).await {
        Ok(Ok(_)) => Some(()),
        _ => None,
    }
}

async fn send_pong(stream: &mut tokio::net::TcpStream, payload: &[u8]) {
    let mut frame = vec![0x8A, 0x80 | payload.len() as u8];
    let mask_key = rand_mask();
    frame.extend_from_slice(&mask_key);
    let masked: Vec<u8> = payload
        .iter()
        .enumerate()
        .map(|(i, b)| b ^ mask_key[i % 4])
        .collect();
    frame.extend_from_slice(&masked);
    let _ = stream.write_all(&frame).await;
}

async fn read_response(stream: &mut tokio::net::TcpStream, buf: &mut [u8]) -> usize {
    let mut total = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let Some(remaining) = deadline.checked_duration_since(tokio::time::Instant::now()) else {
            break total;
        };

        match tokio::time::timeout(remaining, stream.read_exact(&mut buf[total..total + 1])).await {
            Ok(Ok(_)) => {
                total += 1;
                if contains_empty_line(&buf[..total]) {
                    break total;
                }
            }
            Ok(Err(_)) if total > 0 && contains_empty_line(&buf[..total]) => break total,
            Ok(Err(e)) if e.kind() == ErrorKind::WouldBlock => {
                if total > 0 && contains_empty_line(&buf[..total]) {
                    break total;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
                continue;
            }
            Ok(Err(_)) | Err(_) => break total,
        }
    }
}

async fn wait_for_lifecycle_shutdown() -> Vec<&'static str> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let calls = lifecycle().lock().unwrap().clone();
        if calls.contains(&"on_shutdown") {
            return calls;
        }

        if tokio::time::Instant::now() >= deadline {
            return calls;
        }

        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn contains_empty_line(buf: &[u8]) -> bool {
    let s = String::from_utf8_lossy(buf);
    s.contains("\r\n\r\n") || s.contains("\n\n")
}
