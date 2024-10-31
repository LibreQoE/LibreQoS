//! Provides a thread that runs in the background for `lqosd`. It is
//! triggered whenever fresh throughput data is ready to be collected.
//! The data is stored in a "session buffer", to be collated when the
//! collation period timer fires.
//!
//! This is designed to ensure that even long averaging periods don't
//! lose min/max values.

use super::StatsUpdateMessage;
use crate::{
    collector::{
        collation::{collate_stats, StatsSession},
        uisp_ext::gather_uisp_data,
        SESSION_BUFFER,
    },
    submission_queue::{
        comm_channel::{start_communication_channel, SenderChannelMessage},
        enqueue_shaped_devices_if_allowed,
    },
};
use dashmap::DashSet;
use lqos_config::load_config;
use once_cell::sync::Lazy;
use std::{sync::atomic::AtomicU64, time::Duration};
use tokio::sync::mpsc::{self, Receiver, Sender};
use tracing::{info, warn};

static STATS_COUNTER: AtomicU64 = AtomicU64::new(0);
pub(crate) static DEVICE_ID_LIST: Lazy<DashSet<String>> = Lazy::new(DashSet::new);

/// Launches the long-term statistics manager task. Returns immediately,
/// because it creates the channel and then spawns listener threads.
///
/// Returns a channel that may be used to notify of data availability.
pub fn start_long_term_stats() -> Sender<StatsUpdateMessage> {
    let (update_tx, update_rx): (Sender<StatsUpdateMessage>, Receiver<StatsUpdateMessage>) =
        mpsc::channel(102400);
    let (comm_tx, comm_rx): (Sender<SenderChannelMessage>, Receiver<SenderChannelMessage>) =
        mpsc::channel(10);

    let cloned_update_tx = update_tx.clone();
    let _ = std::thread::Builder::new().name("LTS1 Collector".to_string()).spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(startup(update_rx, comm_tx, cloned_update_tx, comm_rx));
    });

    // Return the channel, for notifications
    update_tx
}

async fn startup(
    mut update_rx: Receiver<StatsUpdateMessage>,
    comm_tx: Sender<SenderChannelMessage>,
    update_tx: Sender<StatsUpdateMessage>,
    comm_rx: Receiver<SenderChannelMessage>,
) {
    if let Ok(cfg) = load_config() {
        if !cfg.long_term_stats.gather_stats {
            // Wire up a null recipient to the channel, so it receives messages
            // but doesn't do anything with them.
            tokio::spawn(async move {
                while let Some(_msg) = update_rx.recv().await {
                    // Do nothing
                }
            }).await.unwrap();
            warn!("Long-term stats gathering is disabled in the configuration. Exiting.");
            return;
        }
    }

    let _ = tokio::join!(
        lts_manager(update_rx, comm_tx),
        collation_scheduler(update_tx.clone()),
        uisp_collection_manager(update_tx.clone()),
        start_communication_channel(comm_rx),
    );
    warn!("Long-term stats gathering thread has exited.");
}

async fn collation_scheduler(tx: Sender<StatsUpdateMessage>) {
    info!("Starting collation scheduler");
    loop {
        let collation_period = get_collation_period();
        info!("Collation period: {}s", collation_period.as_secs());
        if tx.send(StatsUpdateMessage::CollationTime).await.is_err() {
            warn!("Unable to send collation time message");
        }
        info!("Sent collation time message. Sleeping.");
        tokio::time::sleep(collation_period).await;
        info!("Collation scheduler woke up.");
    }
}

async fn lts_manager(mut rx: Receiver<StatsUpdateMessage>, comm_tx: Sender<SenderChannelMessage>) {
    info!("Long-term stats gathering thread started");
    loop {
        let msg = rx.recv().await;
        match msg {
            Some(StatsUpdateMessage::ThroughputReady(throughput)) => {
                let counter = STATS_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if counter > 5 {
                    info!("Enqueueing throughput data for collation");
                    SESSION_BUFFER.lock().await.push(StatsSession {
                        throughput: throughput.0,
                        network_tree: throughput.1,
                    });
                }
            }
            Some(StatsUpdateMessage::ShapedDevicesChanged(shaped_devices)) => {
                info!("Enqueueing shaped devices for collation");
                // Update the device id list
                DEVICE_ID_LIST.clear();
                shaped_devices.iter().for_each(|d| {
                    DEVICE_ID_LIST.insert(d.device_id.clone());
                });
                tokio::spawn(enqueue_shaped_devices_if_allowed(
                    shaped_devices,
                    comm_tx.clone(),
                ));
            }
            Some(StatsUpdateMessage::CollationTime) => {
                info!("Collation time reached");
                tokio::spawn(collate_stats(comm_tx.clone()));
            }
            Some(StatsUpdateMessage::UispCollationTime) => {
                info!("UISP Collation time reached");
                tokio::spawn(gather_uisp_data(comm_tx.clone()));
            }
            Some(StatsUpdateMessage::Quit) => {
                // The daemon is exiting, terminate
                let _ = comm_tx.send(SenderChannelMessage::Quit).await;
                break;
            }
            None => {
                warn!("Long-term stats thread received a None message");
            }
        }
    }
}

fn get_collation_period() -> Duration {
    if let Ok(cfg) = load_config() {
        return Duration::from_secs(cfg.long_term_stats.collation_period_seconds.into());
    }

    Duration::from_secs(60)
}

fn get_uisp_collation_period() -> Option<Duration> {
    if let Ok(cfg) = load_config() {
        return Some(Duration::from_secs(
            cfg.long_term_stats.uisp_reporting_interval_seconds.unwrap_or(300),
        ));
    }

    None
}

async fn uisp_collection_manager(control_tx: Sender<StatsUpdateMessage>) {
    // Outer loop: If UISP is disabled, check hourly to see if it
    // was enabled. If it is enabled, start the inner loop.
    loop {
        // Inner loop - if there's a collation period set for UISP,
        // poll it.
        if let Some(period) = get_uisp_collation_period() {
            info!("Starting UISP poller with period {:?}", period);
            loop {
                control_tx
                    .send(StatsUpdateMessage::UispCollationTime)
                    .await
                    .unwrap();
                tokio::time::sleep(period).await;
            }
        } else {
            // Sleep for one hour - then we'll check again
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    }
}
