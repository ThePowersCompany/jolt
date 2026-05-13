use std::sync::{Arc, OnceLock};

use axum::routing::get;
use joltr_core::{
    ws, AuthError, AuthWsJwtLayer, JwtAlgorithm, JwtClaims, JwtConfig, PubSub, PubSubMessage,
    WebSocketHandler, WebSocketSender, WsMessage,
};

const CHAT_CHANNEL: &str = "chat";
const CHAT_JWT_SECRET: &[u8] = b"joltr-basic-chat-dev-secret";

static CHAT_PUBSUB: OnceLock<Arc<PubSub>> = OnceLock::new();

#[derive(Default)]
pub(crate) struct ChatHandler {
    claims: Option<JwtClaims>,
    forwarder: Option<tokio::task::AbortHandle>,
}

impl WebSocketHandler for ChatHandler {
    fn set_claims(&mut self, claims: JwtClaims) {
        self.claims = Some(claims);
    }

    async fn on_open(&mut self, sender: WebSocketSender) {
        self.forwarder = Some(spawn_chat_forwarder(sender));
    }

    async fn on_message(&mut self, msg: WsMessage, _sender: WebSocketSender) {
        let WsMessage::Text(payload) = msg else {
            return;
        };

        chat_pubsub().publish(
            CHAT_CHANNEL,
            PubSubMessage {
                channel: CHAT_CHANNEL.to_string(),
                payload,
                sender_id: self
                    .claims
                    .as_ref()
                    .and_then(|claims| claims.sub.as_ref().cloned()),
            },
        );
    }

    async fn on_close(&mut self) {
        if let Some(forwarder) = self.forwarder.take() {
            forwarder.abort();
        }
    }
}

pub(crate) fn router() -> axum::Router {
    axum::Router::new()
        .route(
            "/ws/chat",
            get(ws!(
                "/ws/chat",
                ChatHandler,
                subprotocol = "joltr-chat-v1",
                auth_fn = chat_auth
            )),
        )
        .layer(AuthWsJwtLayer::new(chat_jwt_config()))
}

fn chat_pubsub() -> Arc<PubSub> {
    Arc::clone(CHAT_PUBSUB.get_or_init(|| Arc::new(PubSub::new())))
}

fn spawn_chat_forwarder(sender: WebSocketSender) -> tokio::task::AbortHandle {
    let mut receiver = chat_pubsub().subscribe(CHAT_CHANNEL);
    let handle = tokio::spawn(async move {
        loop {
            match receiver.recv().await {
                Ok(message) => {
                    if sender.send_json(&message).is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    handle.abort_handle()
}

fn chat_auth(token: &str) -> Result<JwtClaims, AuthError> {
    joltr_utils::jwt::decode(token, &chat_jwt_config())
        .map_err(|err| AuthError::new(err.to_string()))
}

fn chat_jwt_config() -> JwtConfig {
    JwtConfig::new(CHAT_JWT_SECRET.to_vec(), JwtAlgorithm::HS256)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use joltr_core::{jwt_encode, tower::ServiceExt};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn claims(sub: &str) -> JwtClaims {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is sane")
            .as_secs();

        JwtClaims {
            sub: Some(sub.to_string()),
            exp: Some(now + 3600),
            iat: Some(now),
            nbf: None,
            iss: None,
            aud: None,
            custom: Default::default(),
        }
    }

    #[test]
    fn chat_auth_accepts_token_signed_with_example_secret() {
        let token = jwt_encode(&claims("chat-user"), CHAT_JWT_SECRET, JwtAlgorithm::HS256)
            .expect("example token signs");

        let decoded = chat_auth(&token).expect("token validates");

        assert_eq!(decoded.sub.as_deref(), Some("chat-user"));
    }

    #[tokio::test]
    async fn chat_handler_publishes_text_messages_to_pubsub() {
        let mut receiver = chat_pubsub().subscribe(CHAT_CHANNEL);
        let mut handler = ChatHandler::default();
        handler.set_claims(claims("sender-1"));
        let (sender, _rx) = WebSocketSender::channel();

        handler
            .on_message(WsMessage::Text("hello chat".to_string()), sender)
            .await;

        let published = receiver.recv().await.expect("message publishes");
        assert_eq!(published.channel, CHAT_CHANNEL);
        assert_eq!(published.payload, "hello chat");
        assert_eq!(published.sender_id.as_deref(), Some("sender-1"));
    }

    #[tokio::test]
    async fn chat_handler_aborts_forwarder_on_close() {
        let (sender, _rx) = WebSocketSender::channel();
        let mut handler = ChatHandler::default();

        handler.on_open(sender).await;
        assert!(handler.forwarder.is_some());

        handler.on_close().await;
        assert!(handler.forwarder.is_none());
    }

    #[tokio::test]
    async fn chat_router_rejects_missing_ws_jwt_subprotocol() {
        let response = router()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/ws/chat")
                    .body(Body::empty())
                    .expect("request builds"),
            )
            .await
            .expect("router responds");

        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    }
}
