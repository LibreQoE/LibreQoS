//! Long-term stats. This module *gathers* the statistics and makes them
//! available for tools to provide export to other systems - including
//! our own.

mod data_collector;
mod collation_utils;
mod collator;
mod submission;
mod tree;
use log::{info, warn};
use lqos_config::EtcLqos;
use lqos_utils::fdtimer::periodic;
use std::{
  sync::mpsc::{self, Receiver, Sender},
  thread,
};
pub(crate) use submission::{get_stats_totals, get_stats_host, get_stats_tree};

/// Messages to/from the stats collection thread
pub enum StatsMessage {
  /// Fresh throughput stats have been collected
  ThroughputReady,
  /// Request that the stats thread terminate
  Quit,
}

/// Launch the statistics system
pub fn start_long_term_stats() -> Option<Sender<StatsMessage>> {
  if let Ok(cfg) = EtcLqos::load() {
    if let Some(cfg) = cfg.long_term_stats {
      if cfg.gather_stats {
        start_collating_stats(cfg.collation_period_seconds);
        return Some(start_collecting_stats());
      }
    }
  }
  None
}

fn start_collecting_stats() -> Sender<StatsMessage> {
  // Spawn the manager thread, which will wait for message to maintain
  // sync with the generation of stats.
  let (tx, rx): (Sender<StatsMessage>, Receiver<StatsMessage>) =
    mpsc::channel();
  thread::spawn(move || {
    info!("Long-term stats gathering thread started");
    loop {
      let msg = rx.recv();
      match msg {
        Ok(StatsMessage::Quit) => {
          info!("Exiting the long-term stats thread");
          break;
        }
        Ok(StatsMessage::ThroughputReady) => {
          data_collector::gather_throughput_stats();
        }
        Err(e) => {
          warn!("Error in the long-term stats thread message receiver");
          warn!("{e:?}");
        }
      }
    }
  });
  tx
}

fn start_collating_stats(seconds: u32) {
    let interval_ms = (seconds * 1000).into();
    thread::spawn(move || {
        periodic(
            interval_ms, 
            "Long-Term Stats Collation", 
            &mut collator::collate_stats
        );
    });
}