use crate::node_manager::ws::messages::{WsResponse, encode_ws_message};
use crate::node_manager::ws::publish_subscribe::subscriber::Subscriber;
use crate::node_manager::ws::published_channels::PublishedChannels;
use allocative::Allocative;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;

#[derive(Allocative)]
pub(super) struct PublisherChannel {
    pub(super) channel_type: PublishedChannels,
    subscribers: Vec<Subscriber>,
}

impl PublisherChannel {
    pub(super) fn new(t: PublishedChannels) -> Self {
        Self {
            channel_type: t,
            subscribers: vec![],
        }
    }

    pub(super) fn has_subscribers(&self) -> bool {
        !self.subscribers.is_empty()
    }

    pub(super) async fn subscribe(&mut self, sender: Sender<Arc<Vec<u8>>>) {
        self.subscribers.push(Subscriber {
            is_alive: true,
            sender: sender.clone(),
        });
        let welcome = WsResponse::Join {
            channel: self.channel_type,
        };
        if let Ok(payload) = encode_ws_message(&welcome) {
            let _ = sender.send(payload).await;
        }
    }

    /// Submit a message to an entire channel
    pub(super) async fn send(&mut self, message: Arc<Vec<u8>>) {
        for subscriber in self.subscribers.iter_mut() {
            if subscriber.sender.send(message.clone()).await.is_err() {
                subscriber.is_alive = false;
            }
        }
    }

    pub(super) async fn clean(&mut self) {
        self.subscribers.retain(|s| s.is_alive);
    }
}
