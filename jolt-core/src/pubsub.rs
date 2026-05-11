use dashmap::DashMap;
use tokio::sync::broadcast;

/// Default capacity for per-channel broadcast channels.
pub const PUBSUB_BROADCAST_CAPACITY: usize = 256;

/// A message carried over a pub/sub channel.
#[derive(Clone, Debug)]
pub struct PubSubMessage {
    pub channel: String,
    pub payload: String,
    pub sender_id: Option<String>,
}

/// In-memory publish/subscribe hub backed by tokio broadcast channels.
///
/// Each channel name maps to a [broadcast::Sender]; receivers are obtained via
/// [PubSub::subscribe] and publish via [PubSub::publish].
pub struct PubSub {
    channels: DashMap<String, broadcast::Sender<PubSubMessage>>,
}

impl PubSub {
    pub fn new() -> Self {
        Self {
            channels: DashMap::new(),
        }
    }

    pub fn publish(&self, channel: &str, msg: PubSubMessage) -> usize {
        let sender = self
            .channels
            .entry(channel.to_string())
            .or_insert_with(|| broadcast::channel(PUBSUB_BROADCAST_CAPACITY).0);
        match sender.send(msg) {
            Ok(n) => n,
            Err(_) => 0,
        }
    }

    pub fn subscribe(&self, channel: &str) -> broadcast::Receiver<PubSubMessage> {
        let sender = self
            .channels
            .entry(channel.to_string())
            .or_insert_with(|| broadcast::channel(PUBSUB_BROADCAST_CAPACITY).0);
        sender.subscribe()
    }
}
