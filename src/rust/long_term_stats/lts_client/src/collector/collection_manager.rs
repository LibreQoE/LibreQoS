//! Provides a thread that runs in the background for `lqosd`. It is
//! triggered whenever fresh throughput data is ready to be collected.
//! The data is stored in a "session buffer", to be collated when the
//! collation period timer fires.
//!
//! This is designed to ensure that even long averaging periods don't
//! lose min/max values.

use super::StatsUpdateMessage;
use crate::{collector::{collation::{collate_stats, StatsSession}, SESSION_BUFFER}, submission_queue::{enqueue_shaped_devices_if_allowed, comm_channel::{SenderChannelMessage, start_communication_channel}}};
use lqos_config::EtcLqos;
use std::{sync::atomic::AtomicU64, time::Duration};
use tokio::sync::mpsc::{self, Receiver, Sender};

static STATS_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Launches the long-term statistics manager task. Returns immediately,
/// because it creates the channel and then spawns listener threads.
///
/// Returns a channel that may be used to notify of data availability.
pub async fn start_long_term_stats() -> Sender<StatsUpdateMessage> {
    let (update_tx, update_rx): (Sender<StatsUpdateMessage>, Receiver<StatsUpdateMessage>) = mpsc::channel(10);
    let (comm_tx, comm_rx): (Sender<SenderChannelMessage>, Receiver<SenderChannelMessage>) = mpsc::channel(10);

    tokio::spawn(lts_manager(update_rx, comm_tx));
    tokio::spawn(collation_scheduler(update_tx.clone()));
    tokio::spawn(start_communication_channel(comm_rx));

    // Return the channel, for notifications
    update_tx
}

async fn collation_scheduler(tx: Sender<StatsUpdateMessage>) {
    loop {
        let collation_period = get_collation_period();
        tx.send(StatsUpdateMessage::CollationTime).await.unwrap();
        tokio::time::sleep(collation_period).await;
    }
}

async fn lts_manager(mut rx: Receiver<StatsUpdateMessage>, comm_tx: Sender<SenderChannelMessage>) {
    log::info!("Long-term stats gathering thread started");
    loop {
        let msg = rx.recv().await;
        match msg {
            Some(StatsUpdateMessage::ThroughputReady(throughput)) => {
                let counter = STATS_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if counter > 5 {
                    SESSION_BUFFER.lock().await.push(StatsSession {
                        throughput: throughput.0,
                        network_tree: throughput.1,
                    });
                }
            }
            Some(StatsUpdateMessage::ShapedDevicesChanged(shaped_devices)) => {
                tokio::spawn(enqueue_shaped_devices_if_allowed(shaped_devices, comm_tx.clone()));
            }
            Some(StatsUpdateMessage::CollationTime) => {
                tokio::spawn(collate_stats(comm_tx.clone()));
            }
            Some(StatsUpdateMessage::Quit) => {
                // The daemon is exiting, terminate
                let _ = comm_tx.send(SenderChannelMessage::Quit).await;
                break;
            }
            None => {
                log::warn!("Long-term stats thread received a None message");
            }
        }
    }
}

fn get_collation_period() -> Duration {
    if let Ok(cfg) = EtcLqos::load() {
        if let Some(lts) = &cfg.long_term_stats {
            return Duration::from_secs(lts.collation_period_seconds.into());
        }
    }

    Duration::from_secs(60)
}
