//! JOLT-RS-134: WS + pubsub integration test.
//!
//! Two WebSocket clients share a pub/sub hub. Client A sends a text message
//! which the handler publishes to the "chat" channel; client B, subscribed to
//! "chat" via `WebSocketSender::subscribe`, receives the forwarded message.
//!
//! This is an integration test because it requires a running axum server with
//! both `AuthWsJwtLayer` and `PubSub` wired in — the `ws!` macro does not
//! (yet) inject a pubsub handle into the generated sender, so this test
//! constructs the WS upgrade handler manually.

use std::io::ErrorKind;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::ws::{WebSocket, WebSocketUpgrade};
use axum::routing::get;
use axum::Extension;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use jolt_core::{
    AuthWsJwtLayer, PubSub, PubSubMessage, Subscription, WebSocketHandler,
    WebSocketSender, WsMessage,
};
use jolt_utils::jwt::{JwtClaims, JwtConfig};

/// Handler that subscribes to a pub/sub channel on open and publishes every
/// received text message to that channel. Both client A and client B share
/// the same underlying `Arc<PubSub>`, so a message published by one connection
/// is forwarded to all subscribers.
struct PubSubChatHandler {
    pubsub: Arc<PubSub>,
    sub: Option<Subscription>,
}

impl WebSocketHandler for PubSubChatHandler {
    async fn on_open(&mut self, sender: WebSocketSender) {
        self.sub = sender.subscribe("chat");
    }

    async fn on_message(&mut self, msg: WsMessage, _sender: WebSocketSender) {
        if let WsMessage::Text(text) = &msg {
            self.pubsub.publish(
                "chat",
                PubSubMessage {
                    channel: "chat".to_string(),
                    payload: text.clone(),
                    sender_id: None,
                },
            );
        }
    }
}

#[tokio::test]
async fn two_clients_client_a_publishes_client_b_receives() {
    let _ = tracing_subscriber::fmt().try_init();

    let secret = b"jolt-134-test-secret";
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as usize;

    let claims = JwtClaims {
        sub: "user-134".to_owned(),
        exp: now + 3600,
        iat: Some(now),
        extra: Default::default(),
    };
    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret),
    )
    .expect("HS256 encoding of valid claims must succeed");

    let config = JwtConfig::new(secret.to_vec(), Algorithm::HS256);
    let pubsub = Arc::new(PubSub::new());

    let pubsub_for_route = Arc::clone(&pubsub);
    let app = axum::Router::new()
        .route(
            "/ws",
            get(
                move |ws: WebSocketUpgrade,
                      Extension(extracted_claims): Extension<JwtClaims>| {
                    let pubsub = Arc::clone(&pubsub_for_route);
                    async move {
                        ws.on_upgrade(move |socket: WebSocket| {
                            let pubsub = Arc::clone(&pubsub);
                            async move {
                                let (mut tx, mut rx) = socket.split();
                                let (sender, writer_rx) =
                                    WebSocketSender::channel_with_pubsub(
                                        Arc::clone(&pubsub),
                                    );

                                let writer = tokio::spawn(async move {
                                    let mut writer_rx = writer_rx;
                                    use futures_util::SinkExt;
                                    while let Some(msg) = writer_rx.recv().await
                                    {
                                        if tx.send(msg).await.is_err() {
                                            break;
                                        }
                                    }
                                });

                                let mut handler = PubSubChatHandler {
                                    pubsub,
                                    sub: None,
                                };
                                handler.set_claims(extracted_claims);
                                handler.on_open(sender.clone()).await;
                                handler.on_ready(sender.clone()).await;

                                use futures_util::StreamExt;
                                while let Some(Ok(msg)) = rx.next().await {
                                    let ws_msg = WsMessage::from(msg);
                                    let is_close =
                                        matches!(&ws_msg, WsMessage::Close(_));
                                    handler
                                        .on_message(ws_msg, sender.clone())
                                        .await;
                                    if is_close {
                                        break;
                                    }
                                }
                                handler.on_close().await;
                                writer.abort();
                                handler.on_shutdown().await;
                            }
                        })
                    }
                },
            ),
        )
        .layer(AuthWsJwtLayer::new(config));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    // Connect client A and B.
    let mut stream_a = tokio::net::TcpStream::connect(addr).await.unwrap();
    ws_upgrade(&mut stream_a, addr, &token).await;

    let mut stream_b = tokio::net::TcpStream::connect(addr).await.unwrap();
    ws_upgrade(&mut stream_b, addr, &token).await;

    // Wait for both handlers to complete on_open (subscribe + forward task
    // spawn). The subscription task needs a tick to register its broadcast
    // receiver.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Client A sends a text message. The handler receives it in on_message,
    // publishes it to the "chat" channel. Client B's subscription forward task
    // serializes the PubSubMessage as JSON and sends it over the WebSocket.
    send_text_frame(&mut stream_a, "hello from A").await;

    // Client B receives the forwarded JSON message.
    let msg_on_b = recv_frame_timeout(&mut stream_b).await;
    assert!(
        !msg_on_b.is_empty(),
        "client B must receive a forwarded message from pubsub"
    );

    let parsed: serde_json::Value =
        serde_json::from_str(&msg_on_b).expect("forwarded pubsub message must be valid JSON");
    assert_eq!(
        parsed["channel"], "chat",
        "forwarded message channel must be 'chat'"
    );
    assert_eq!(
        parsed["payload"], "hello from A",
        "forwarded message payload must match what client A sent"
    );
    assert!(
        parsed["sender_id"].is_null(),
        "sender_id must be null when published by server handler"
    );

    // Client A also receives the echo (both are subscribed). Drain it.
    let _echo_on_a = recv_frame_timeout(&mut stream_a).await;

    // Clean shutdown.
    send_close_frame(&mut stream_a).await;
    send_close_frame(&mut stream_b).await;
    let _ = recv_frame_raw_timeout(&mut stream_a).await;
    let _ = recv_frame_raw_timeout(&mut stream_b).await;
}

// -- WS frame helpers -------------------------------------------------------

async fn ws_upgrade(
    stream: &mut tokio::net::TcpStream,
    addr: std::net::SocketAddr,
    token: &str,
) {
    let request = format!(
        "GET /ws HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Sec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Protocol: jolt-jwt, {token}\r\n\
         \r\n"
    );
    stream.write_all(request.as_bytes()).await.unwrap();

    let mut buf = vec![0u8; 4096];
    let n = read_response(stream, &mut buf).await;
    let response = String::from_utf8_lossy(&buf[..n]);
    assert!(
        response.starts_with("HTTP/1.1 101"),
        "expected 101 Switching Protocols, got: {response}"
    );
}

async fn send_text_frame(stream: &mut tokio::net::TcpStream, payload: &str) {
    let frame = build_masked_frame(0x1, payload.as_bytes());
    stream.write_all(&frame).await.unwrap();
}

async fn send_close_frame(stream: &mut tokio::net::TcpStream) {
    let frame = build_masked_frame(0x8, &[]);
    stream.write_all(&frame).await.unwrap();
}

fn build_masked_frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let len = payload.len();
    let mut frame = Vec::new();
    frame.push(0x80 | opcode);

    if len < 126 {
        frame.push(0x80 | len as u8);
    } else if len <= 0xFFFF {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        frame.push(0x80 | 127);
        frame.extend_from_slice(&(len as u64).to_be_bytes());
    }

    let mask_key = rand_mask();
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
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    [t as u8, (t >> 8) as u8, (t >> 16) as u8, (t >> 24) as u8]
}

/// Receives a text frame from the server (unmasked per RFC 6455) with a 5 s
/// timeout. Returns the decoded UTF-8 payload or an empty string on timeout.
async fn recv_frame_timeout(stream: &mut tokio::net::TcpStream) -> String {
    let mut buf = vec![0u8; 8192];
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if tokio::time::Instant::now() > deadline {
            return String::new();
        }
        let bytes = match stream.read(&mut buf).await {
            Ok(0) => return String::new(),
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                tokio::time::sleep(Duration::from_millis(10)).await;
                continue;
            }
            Err(_) => return String::new(),
        };
        if bytes >= 2 {
            let opcode = buf[0] & 0x0F;
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

            let mask = (buf[1] & 0x80) != 0;
            if !mask {
                if bytes < offset + payload_len {
                    continue;
                }
                return String::from_utf8_lossy(&buf[offset..offset + payload_len])
                    .to_string();
            }

            if bytes < offset + 4 + payload_len {
                continue;
            }
            let mask_key = &buf[offset..offset + 4];
            let mask_len = payload_len;
            offset += 4;
            if bytes < offset + mask_len {
                continue;
            }

            match opcode {
                0x1 => {
                    let data: Vec<u8> = buf[offset..offset + mask_len]
                        .iter()
                        .enumerate()
                        .map(|(i, b)| b ^ mask_key[i % 4])
                        .collect();
                    return String::from_utf8_lossy(&data).to_string();
                }
                0x8 => {
                    // Close frame — stop trying to read text.
                    return String::new();
                }
                0x9 => {
                    let data: Vec<u8> = buf[offset..offset + mask_len]
                        .iter()
                        .enumerate()
                        .map(|(i, b)| b ^ mask_key[i % 4])
                        .collect();
                    send_pong(stream, &data).await;
                }
                _ => {}
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Receives a close frame from the server with a 3 s timeout.
async fn recv_frame_raw_timeout(stream: &mut tokio::net::TcpStream) -> Option<Vec<u8>> {
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
                if (buf[1] & 0x80) != 0 {
                    // Masked close from client — skip mask bytes.
                    offset += 4;
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
            _ => return None,
        }
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

// -- HTTP response reader ---------------------------------------------------

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
