use crate::throughput_tracker::THROUGHPUT_TRACKER;
use once_cell::sync::Lazy;
use std::{
  net::IpAddr,
  sync::{atomic::AtomicU64, Mutex},
};

static SUBMISSION_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) struct StatsSession {
  pub(crate) bits_per_second: (u64, u64),
  pub(crate) packets_per_second: (u64, u64),
  pub(crate) shaped_bits_per_second: (u64, u64),
  pub(crate) hosts: Vec<SessionHost>,
}

pub(crate) struct SessionHost {
  pub(crate) circuit_id: String,
  pub(crate) ip_address: IpAddr,
  pub(crate) bits_per_second: (u64, u64),
  pub(crate) median_rtt: f32,
}

pub(crate) static SESSION_BUFFER: Lazy<Mutex<Vec<StatsSession>>> =
  Lazy::new(|| Mutex::new(Vec::new()));

pub(crate) fn gather_throughput_stats() {
  let count =
    SUBMISSION_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
  if count < 5 {
    // Ignore the first few sets of data
    return;
  }

  // Gather Global Stats
  let packets_per_second = (
    THROUGHPUT_TRACKER
      .packets_per_second
      .0
      .load(std::sync::atomic::Ordering::Relaxed),
    THROUGHPUT_TRACKER
      .packets_per_second
      .1
      .load(std::sync::atomic::Ordering::Relaxed),
  );
  let bits_per_second = THROUGHPUT_TRACKER.bits_per_second();
  let shaped_bits_per_second = THROUGHPUT_TRACKER.shaped_bits_per_second();

  let mut session = StatsSession {
    bits_per_second,
    shaped_bits_per_second,
    packets_per_second,
    hosts: Vec::with_capacity(THROUGHPUT_TRACKER.raw_data.len()),
  };

  THROUGHPUT_TRACKER
    .raw_data
    .iter()
    .filter(|t| t.circuit_id.is_some())
    .for_each(|tp| {
      let bytes_per_second = tp.bytes_per_second;
      let bits_per_second = (bytes_per_second.0 * 8, bytes_per_second.1 * 8);
      session.hosts.push(SessionHost {
        circuit_id: tp.circuit_id.as_ref().unwrap().clone(),
        ip_address: tp.key().as_ip(),
        bits_per_second,
        median_rtt: tp.median_latency(),
      });
    });

  SESSION_BUFFER.lock().unwrap().push(session);
}
