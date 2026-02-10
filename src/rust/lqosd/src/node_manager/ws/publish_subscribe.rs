//! Builds a PubSub (Publisher-Subscriber) system, tailored
//! to what LibreQoS needs for its node manager. This isn't
//! intended to be particularly reusable.

mod publisher_channel;
mod subscriber;

use crate::node_manager::ws::messages::{WsResponse, encode_ws_message};
use crate::node_manager::ws::publish_subscribe::publisher_channel::PublisherChannel;
use crate::node_manager::ws::published_channels::PublishedChannels;
use arc_swap::ArcSwap;
use fxhash::FxHashSet;
use std::sync::Arc;
use strum::IntoEnumIterator;
use tokio::sync::Mutex;
use tokio::sync::mpsc::Sender;
use tracing::warn;

/// Represents a PubSub structure intended to be wrapped in
/// an Arc, and used from within the websocket system.
///
/// Wrapping in an Arc means there is only ever one of them.
/// Creating a second is a *terrible* idea.
/// Please don't.
pub struct PubSub {
    channels: Mutex<Vec<PublisherChannel>>,
    living_channels: ArcSwap<FxHashSet<PublishedChannels>>,
}

impl PubSub {
    /// Constructs a new PubSub interface with a default set of
    /// channels.
    pub(super) fn new() -> Arc<Self> {
        let mut channels = Vec::new();
        for c in PublishedChannels::iter() {
            channels.push(PublisherChannel::new(c));
        }

        let result = Self {
            channels: Mutex::new(channels),
            living_channels: ArcSwap::new(Arc::new(FxHashSet::default())),
        };
        Arc::new(result)
    }

    /// Adds a subscriber to a channel set. Once added, they are
    /// self-managing and will be deleted when they become inactive
    /// automatically.
    pub(super) async fn subscribe(&self, channel: PublishedChannels, sender: Sender<Arc<Vec<u8>>>) {
        let mut channels = self.channels.lock().await;
        if let Some(channel) = channels.iter_mut().find(|c| c.channel_type == channel) {
            channel.subscribe(sender).await;
        } else {
            warn!(
                "Tried to subscribe to channel {:?}, which doesn't exist",
                channel
            );
        }
    }

    pub(super) async fn unsubscribe(
        &self,
        channel: PublishedChannels,
        sender: Sender<Arc<Vec<u8>>>,
    ) {
        let mut channels = self.channels.lock().await;
        if let Some(channel) = channels.iter_mut().find(|c| c.channel_type == channel) {
            channel.unsubscribe(&sender);
        }
    }

    /// Provide a set of channels that have subscribers.
    pub(super) async fn update_living_channel_list(&self) {
        let channels = self.channels.lock().await;
        let living_channels: FxHashSet<PublishedChannels> = channels
            .iter()
            .filter(|c| c.has_subscribers())
            .map(|c| c.channel_type)
            .collect();
        self.living_channels.store(Arc::new(living_channels));
    }

    /// Checks that a channel has anyone listening for it. If it doesn't,
    /// there's no point in using CPU to process it!
    pub(super) async fn is_channel_alive(&self, channel: PublishedChannels) -> bool {
        self.living_channels.load().contains(&channel)
    }

    /// Sends a message to everyone subscribed to a topic. If senders' channels
    /// are dead, they are removed from the list.
    pub(super) async fn send(&self, channel: PublishedChannels, message: WsResponse) {
        let payload = match encode_ws_message(&message) {
            Ok(payload) => payload,
            Err(err) => {
                warn!("Failed to encode ws message for {:?}: {:?}", channel, err);
                return;
            }
        };
        let mut channels = self.channels.lock().await;
        if let Some(channel) = channels.iter_mut().find(|c| c.channel_type == channel) {
            channel.send(payload).await;
        }
    }

    pub(super) async fn clean(&self) {
        let mut channels = self.channels.lock().await;
        for c in channels.iter_mut() {
            c.clean().await;
        }
    }
}
