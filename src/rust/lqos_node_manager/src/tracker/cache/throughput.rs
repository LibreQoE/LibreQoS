use lazy_static::*;
use parking_lot::RwLock;
use rocket::serde::Serialize;

lazy_static! {
    /// Global storage of the current throughput counter.
    pub static ref CURRENT_THROUGHPUT : RwLock<ThroughputPerSecond> = RwLock::new(ThroughputPerSecond::default());
}

lazy_static! {
    /// Global storage of the last N seconds throughput buffer.
    pub static ref THROUGHPUT_BUFFER : RwLock<ThroughputRingbuffer> = RwLock::new(ThroughputRingbuffer::new());
}

/// Stores total system throughput per second.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(crate = "rocket::serde")]
pub struct ThroughputPerSecond {
    pub bits_per_second: (u64, u64),
    pub packets_per_second: (u64, u64),
    pub shaped_bits_per_second: (u64, u64),
}

impl Default for ThroughputPerSecond {
    fn default() -> Self {
        Self {
            bits_per_second: (0,0),
            packets_per_second: (0,0),
            shaped_bits_per_second: (0, 0),
        }
    }
}

/// How many entries (at one per second) should we keep in the
/// throughput ringbuffer?
const RINGBUFFER_SAMPLES: usize = 300;

/// Stores Throughput samples in a ringbuffer, continually
/// updating. There are always RINGBUFFER_SAMPLES available,
/// allowing for non-allocating/non-growing storage of
/// throughput for the dashboard summaries.
pub struct ThroughputRingbuffer {
    readings: Vec<ThroughputPerSecond>,
    next: usize,
}

impl ThroughputRingbuffer {
    fn new() -> Self {
        Self {
            readings: vec![ThroughputPerSecond::default(); RINGBUFFER_SAMPLES],
            next: 0,
        }
    }

    pub fn store(&mut self, reading: ThroughputPerSecond) {
        self.readings[self.next] = reading;
        self.next += 1;
        self.next %= RINGBUFFER_SAMPLES;
    }

    pub fn get_result(&self) -> Vec<ThroughputPerSecond> {
        let mut result = Vec::new();

        for i in self.next .. RINGBUFFER_SAMPLES {
            result.push(self.readings[i]);
        }
        for i in 0..self.next {
            result.push(self.readings[i]);
        }

        result
    }
}
