use crate::tracker::ThroughputPerSecond;
use lqos_bus::{bus_request, BusRequest, BusResponse};
use once_cell::sync::Lazy;
use rocket::tokio::sync::RwLock;

pub static THROUGHPUT_BUFFER: Lazy<RwLock<TotalThroughput>> =
  Lazy::new(|| RwLock::new(TotalThroughput::new()));

/// Maintains an in-memory ringbuffer of the last 5 minutes of
/// throughput data.
pub struct TotalThroughput {
  data: Vec<ThroughputPerSecond>,
  head: usize,
  prev_head: usize,
}

impl TotalThroughput {
  /// Create a new throughput ringbuffer system
  pub fn new() -> Self {
    Self {
      data: vec![ThroughputPerSecond::default(); 300],
      head: 0,
      prev_head: 0,
    }
  }

  /// Run once per second to update the ringbuffer with current data
  pub async fn tick(&mut self) {
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
          self.data[self.head].bits_per_second = bits_per_second;
          self.data[self.head].packets_per_second = packets_per_second;
          self.data[self.head].shaped_bits_per_second = shaped_bits_per_second;
          self.prev_head = self.head;
          self.head += 1;
          self.head %= 300;
        }
      }
    }
  }

  /// Retrieve just the current throughput data (1 tick)
  pub fn current(&self) -> ThroughputPerSecond {
    self.data[self.prev_head]
  }

  /// Retrieve the head (0-299) and the full current throughput
  /// buffer. Used to populate the dashboard the first time.
  pub fn copy(&self) -> (usize, Vec<ThroughputPerSecond>) {
    (self.head, self.data.clone())
  }
}
