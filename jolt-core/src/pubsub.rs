use dashmap::DashMap;
use tokio::sync::broadcast;

/// Default capacity for per-channel broadcast channels.
pub const PUBSUB_BROADCAST_CAPACITY: usize = 256;

/// A message carried over a pub/sub channel.
pub struct PubSubMessage;

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
}
