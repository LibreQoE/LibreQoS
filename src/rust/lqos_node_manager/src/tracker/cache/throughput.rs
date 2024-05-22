use std::sync::Mutex;

use crate::tracker::ThroughputPerSecond;
use lqos_bus::{bus_request, BusRequest, BusResponse};
use once_cell::sync::Lazy;

pub static THROUGHPUT_BUFFER: Lazy<TotalThroughput> =
  Lazy::new(|| TotalThroughput::new());

/// Maintains an in-memory ringbuffer of the last 5 minutes of
/// throughput data.
pub struct TotalThroughput {
  inner: Mutex<TotalThroughputInner>
}

struct TotalThroughputInner {
  data: Vec<ThroughputPerSecond>,
  head: usize,
  prev_head: usize,
}

impl TotalThroughput {
  /// Create a new throughput ringbuffer system
  pub fn new() -> Self {
    Self {
      inner: Mutex::new(TotalThroughputInner {
        data: vec![ThroughputPerSecond::default(); 300],
        head: 0,
        prev_head: 0,
      }),
    }
  }

  /// Run once per second to update the ringbuffer with current data
  pub async fn tick(&self) {
    if let Ok(messages) =
      bus_request(vec![BusRequest::GetCurrentThroughput]).await
    {
      for msg in messages {
        if let BusResponse::CurrentThroughput {
          bits_per_second,
          packets_per_second,
          shaped_bits_per_second,
        } = msg
        {
          let mut lock = self.inner.lock().unwrap();
          let head = lock.head;
          lock.data[head].bits_per_second = bits_per_second;
          lock.data[head].packets_per_second = packets_per_second;
          lock.data[head].shaped_bits_per_second = shaped_bits_per_second;
          lock.prev_head = lock.head;
          lock.head += 1;
          lock.head %= 300;
        }
      }
    }
  }

  /// Retrieve just the current throughput data (1 tick)
  pub fn current(&self) -> ThroughputPerSecond {
    let lock = self.inner.lock().unwrap();
    lock.data[lock.prev_head]
  }

  /// Retrieve the head (0-299) and the full current throughput
  /// buffer. Used to populate the dashboard the first time.
  pub fn copy(&self) -> (usize, Vec<ThroughputPerSecond>) {
    let lock = self.inner.lock().unwrap();
    (lock.head, lock.data.clone())
  }
}
