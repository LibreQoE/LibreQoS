//! Builds a PubSub (Publisher-Subscriber) system, tailored
//! to what LibreQoS needs for its node manager. This isn't
//! intended to be particularly reusable.

mod publisher_channel;
mod subscriber;

use std::sync::Arc;
use strum::IntoEnumIterator;
use tokio::sync::mpsc::Sender;
use tokio::sync::Mutex;
use crate::node_manager::ws::publish_subscribe::publisher_channel::PublisherChannel;
use crate::node_manager::ws::published_channels::PublishedChannels;

/// Represents a PubSub structure intended to be wrapped in
/// an Arc, and used from within the websocket system.
///
/// Wrapping in an Arc means there is only ever one of them.
/// Creating a second is a *terrible* idea.
/// Please don't.
pub struct PubSub {
    channels: Mutex<Vec<PublisherChannel>>,
}

impl PubSub {
    /// Constructs a new PubSub interface with a default set of
    /// channels.
    pub(super) fn new() -> Arc<Self> {
        let mut channels = Vec::new();
        for c in PublishedChannels::iter() {
            channels.push(
                PublisherChannel::new(c)
            );
        }

        let result = Self {
            channels: Mutex::new(channels),
        };
        Arc::new(result)
    }

    /// Adds a subscriber to a channel set. Once added, they are
    /// self-managing and will be deleted when they become inactive
    /// automatically.
    pub(super) async fn subscribe(&self, channel: PublishedChannels, sender: Sender<String>) {
        let mut channels = self.channels.lock().await;
        if let Some(channel) = channels.iter_mut().find(|c| c.channel_type == channel) {
            channel.subscribe(sender).await;
        } else {
            log::warn!("Tried to subscribe to channel {:?}, which doesn't exist", channel);
        }
    }

    /// Checks that a channel has anyone listening for it. If it doesn't,
    /// there's no point in using CPU to process it!
    pub(super) async fn is_channel_alive(&self, channel: PublishedChannels) -> bool {
        let channels = self.channels.lock().await;
        if let Some(channel) = channels.iter().find(|c| c.channel_type == channel) {
            channel.has_subscribers()
        } else {
            false
        }
    }

    /// Sends a message to everyone subscribed to a topic. If senders' channels
    /// are dead, they are removed from the list.
    pub(super) async fn send(&self, channel: PublishedChannels, message: String) {
        let mut channels = self.channels.lock().await;
        if let Some(channel) = channels.iter_mut().find(|c| c.channel_type == channel) {
            channel.send_and_clean(message).await;
        }
    }
}