use serde_json::json;
use tokio::sync::mpsc::Sender;
use crate::node_manager::ws::publish_subscribe::subscriber::Subscriber;
use crate::node_manager::ws::published_channels::PublishedChannels;

pub(super) struct PublisherChannel {
    pub(super) channel_type: PublishedChannels,
    subscribers: Vec<Subscriber>
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
    
    pub(super) async fn subscribe(&mut self, sender: Sender<String>) {
        self.subscribers.push(Subscriber{
            is_alive: true,
            sender: sender.clone(),
        });
        let welcome = json!(
            {
                "event" : "join",
                "channel" : self.channel_type.to_string(),
            }
        ).to_string();
        let _ = sender.send(welcome).await;
    }

    /// Submit a message to an entire channel
    pub(super) async fn send(&mut self, message: String) {
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