use crate::tracker::ThroughputPerSecond;
use lqos_bus::{bus_request, BusRequest, BusResponse};
use once_cell::sync::Lazy;
use rocket::tokio::sync::RwLock;

const THROUGHPUT_BUFFER_SIZE: usize = 300;

pub static THROUGHPUT_BUFFER: Lazy<TotalThroughput> = Lazy::new(TotalThroughput::new);

pub struct TotalThroughput {
  inner: RwLock<TotalThroughputInner>
}

impl TotalThroughput {
  fn new() -> Self {
    TotalThroughput { inner: RwLock::new(TotalThroughputInner::new()) }
  }

  pub async fn tick(&self) {
    let mut lock = self.inner.write().await;
    lock.tick().await;
  }

  pub async fn current(&self) -> ThroughputPerSecond {
    self.inner.read().await.current()
  }

  pub async fn copy(&self) -> (usize, Vec<ThroughputPerSecond>) {
    self.inner.read().await.copy()
  }
}


/// Maintains an in-memory ringbuffer of the last 5 minutes of
/// throughput data.
struct TotalThroughputInner {
  data: Vec<ThroughputPerSecond>,
  head: usize,
  prev_head: usize,
}

impl TotalThroughputInner {
  /// Create a new throughput ringbuffer system
  fn new() -> Self {
    Self {
      data: vec![ThroughputPerSecond::default(); THROUGHPUT_BUFFER_SIZE],
      head: 0,
      prev_head: 0,
    }
  }

  /// Run once per second to update the ringbuffer with current data
  async fn tick(&mut self) {
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
  fn current(&self) -> ThroughputPerSecond {
    self.data[self.prev_head]
  }

  /// Retrieve the head (0-299) and the full current throughput
  /// buffer. Used to populate the dashboard the first time.
  fn copy(&self) -> (usize, Vec<ThroughputPerSecond>) {
    (self.head, self.data.clone())
  }
}
