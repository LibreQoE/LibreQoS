//! Strongly-typed RTT data.

use allocative_derive::Allocative;
use serde::Serialize;
use zerocopy::FromBytes;

/// RTT value, stored as nanoseconds since this is the unit produced by the
/// eBPF/XDP code paths.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Allocative,
    Hash,
    FromBytes,
    Default,
)]
#[repr(C)]
pub struct RttData {
    nanoseconds: u64,
}

#[allow(dead_code)]
impl RttData {
    /// Create an RTT value from nanoseconds.
    pub fn from_nanos(nanoseconds: u64) -> Self {
        Self { nanoseconds }
    }

    /// Return the RTT in nanoseconds.
    pub const fn as_nanos(&self) -> u64 {
        self.nanoseconds
    }

    /// Return the RTT in microseconds.
    pub fn as_micros(&self) -> f64 {
        self.nanoseconds as f64 / 1_000.0
    }

    /// Return the RTT in milliseconds.
    pub fn as_millis(&self) -> f64 {
        self.nanoseconds as f64 / 1_000_000.0
    }

    /// Return the RTT in milliseconds * 100.
    pub fn as_millis_times_100(&self) -> f64 {
        self.nanoseconds as f64 / 10_000.0
    }

    /// Return the RTT in seconds.
    pub fn as_seconds(&self) -> f64 {
        self.nanoseconds as f64 / 1_000_000_000.0
    }
}

