//! Long-term stats. This module *gathers* the statistics and makes them
//! available for tools to provide export to other systems - including
//! our own.

mod data_collector;
mod collation_utils;
mod collator;
mod submission;
mod tree;
mod licensing;
mod lts_queue;
mod pki;
use std::time::Duration;
use log::{info, warn};
use lqos_config::EtcLqos;
pub(crate) use submission::{get_stats_totals, get_stats_host, get_stats_tree};
use tokio::{sync::mpsc::{Sender, Receiver, self}, time::Instant};

#[derive(Debug)]
/// Messages to/from the stats collection thread
pub enum StatsMessage {
  /// Fresh throughput stats have been collected
  ThroughputReady,
}

/// Launch the statistics system
pub async fn start_long_term_stats() -> Option<Sender<StatsMessage>> {
  if let Ok(cfg) = EtcLqos::load() {
    if let Some(cfg) = cfg.long_term_stats {
      if cfg.gather_stats {
        start_collating_stats(cfg.collation_period_seconds).await;
        return Some(start_collecting_stats().await);
      } else {
        log::warn!("Long-term stats 'gather_stats' set to false");
      }
    }
  }
  log::warn!("Not gathering long-term stats. Check the [long_term_stats] section of /etc/lqos.conf.");
  None
}

async fn start_collecting_stats() -> Sender<StatsMessage> {
  // Spawn the manager thread, which will wait for message to maintain
  // sync with the generation of stats.
  let (tx, rx): (Sender<StatsMessage>, Receiver<StatsMessage>) = mpsc::channel(10);
  tokio::spawn(long_term_stats_collector(rx));
  tx
}

async fn long_term_stats_collector(mut rx: Receiver<StatsMessage>) {
  info!("Long-term stats gathering thread started");
    loop {
      let msg = rx.recv().await;
      match msg {
        Some(StatsMessage::ThroughputReady) => {
          data_collector::gather_throughput_stats().await;
        }
        None => {
          warn!("Long-term stats thread received a None message");
        }
      }
    }
}

async fn start_collating_stats(seconds: u32) {
    tokio::spawn(collation_task(seconds));
}

async fn collation_task(interval_seconds: u32) {
  loop {
    let now = Instant::now();
    collator::collate_stats().await;
    let elapsed = now.elapsed();
    let sleep_time = Duration::from_secs(interval_seconds.into()) - elapsed;
    if sleep_time.as_secs() > 0 {
      tokio::time::sleep(sleep_time).await;
    }
  }
}