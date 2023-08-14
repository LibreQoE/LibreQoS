use std::sync::RwLock;
use once_cell::sync::Lazy;
use tokio::sync::mpsc::Sender;
use crate::transport_data::StatsSubmission;
use super::{queue::enqueue_if_allowed, comm_channel::SenderChannelMessage};

pub(crate) static CURRENT_STATS: Lazy<RwLock<Option<StatsSubmission>>> = Lazy::new(|| RwLock::new(None));

pub(crate) async fn new_submission(data: StatsSubmission, comm_tx: Sender<SenderChannelMessage>) {
    *CURRENT_STATS.write().unwrap() = Some(data.clone());
    enqueue_if_allowed(data, comm_tx).await;
}

pub fn get_current_stats() -> Option<StatsSubmission> {
    CURRENT_STATS.read().unwrap().clone()
}