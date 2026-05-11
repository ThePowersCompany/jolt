use std::io::ErrorKind;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::routing::get;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use joltr_core::{ws, AuthError, JwtClaims, WebSocketHandler, WebSocketSender, WsMessage};

static RECEIVED: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

fn received() -> &'static Mutex<Vec<String>> {
    RECEIVED.get_or_init(|| Mutex::new(Vec::new()))
}

#[derive(Default)]
struct EchoHandler;

impl WebSocketHandler for EchoHandler {
    async fn on_open(&mut self, sender: WebSocketSender) {
        let _ = sender.send_text("welcome");
    }

    async fn on_message(&mut self, msg: WsMessage, sender: WebSocketSender) {
        match &msg {
            WsMessage::Text(text) => {
                received().lock().unwrap().push(text.clone());
                let _ = sender.send_text(text);
            }
            WsMessage::Close(_) => {}
            _ => {}
        }
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
    let _ = tracing_subscriber::fmt().try_init();
    received().lock().unwrap().clear();

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
        first_frame,
        "welcome",
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

    send_close_frame(&mut stream).await;

    let close_frame = recv_frame_raw(&mut stream).await;
    assert!(
        close_frame.is_none() || close_frame == Some(vec![0x88, 0x00]),
        "expected close frame from server after client close"
    );
}

#[tokio::test]
async fn invalid_token_is_rejected_with_401() {
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
    [
        (t >> 24) as u8,
        (t >> 16) as u8,
        (t >> 8) as u8,
        t as u8,
    ]
}

async fn recv_frame(stream: &mut tokio::net::TcpStream) -> String {
    let mut buf = vec![0u8; 8192];

    loop {
        let bytes = match stream.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                tokio::time::sleep(Duration::from_millis(10)).await;
                continue;
            }
            Err(_) => break,
        };

        if bytes >= 2 {
            let opcode = buf[0] & 0x0F;
            let mask = (buf[1] & 0x80) != 0;
            let mut payload_len = (buf[1] & 0x7F) as usize;
            let mut offset = 2;

            if payload_len == 126 {
                if bytes < 4 {
                    continue;
                }
                payload_len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
                offset = 4;
            } else if payload_len == 127 {
                if bytes < 10 {
                    continue;
                }
                payload_len = u64::from_be_bytes([
                    buf[2], buf[3], buf[4], buf[5], buf[6], buf[7], buf[8], buf[9],
                ]) as usize;
                offset = 10;
            }

            if mask {
                if bytes < offset + 4 + payload_len {
                    continue;
                }
                let _mask_key = &buf[offset..offset + 4];
                offset += 4;
            }

            if bytes < offset + payload_len {
                continue;
            }

            let data = &buf[offset..offset + payload_len];

            match opcode {
                0x1 => return String::from_utf8_lossy(data).to_string(),
                0x9 => {
                    send_pong(stream, data).await;
                    continue;
                }
                _ => continue,
            }
        }

        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    String::new()
}

async fn recv_frame_raw(stream: &mut tokio::net::TcpStream) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; 8192];

    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        if tokio::time::Instant::now() > deadline {
            return None;
        }

        match stream.read(&mut buf).await {
            Ok(0) => return None,
            Ok(n) if n >= 2 => {
                let opcode = buf[0] & 0x0F;
                let mut payload_len = (buf[1] & 0x7F) as usize;
                let mut offset = 2;

                if payload_len == 126 {
                    if n < 4 {
                        continue;
                    }
                    payload_len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
                    offset = 4;
                } else if payload_len == 127 {
                    if n < 10 {
                        continue;
                    }
                    payload_len = u64::from_be_bytes([
                        buf[2], buf[3], buf[4], buf[5], buf[6], buf[7], buf[8], buf[9],
                    ]) as usize;
                    offset = 10;
                }

                return match opcode {
                    0x8 => Some(buf[..offset + payload_len].to_vec()),
                    _ => None,
                };
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                tokio::time::sleep(Duration::from_millis(10)).await;
                continue;
            }
            _ => {}
        }

        tokio::time::sleep(Duration::from_millis(10)).await;
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
