//! Provides a set of types for representing round-trip time (RTT) data,
//! as produced by the eBPF system and consumed in different ways.
//! 
//! Adopting strong-typing is an attempt to reduce confusion with
//! multipliers, divisors, etc. It is intended to become pervasive
//! throughout the system.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct RttData {
    nanoseconds: u64,
}

#[allow(dead_code)]
impl RttData {
    pub fn from_nanos(nanoseconds: u64) -> Self {
        Self { nanoseconds }
    }

    pub fn as_nanos(&self) -> u64 {
        self.nanoseconds
    }

    pub fn as_micros(&self) -> f64 {
        self.nanoseconds as f64 / 1_000.0
    }

    pub fn as_millis(&self) -> f64 {
        self.nanoseconds as f64 / 1_000_000.0
    }

    pub fn as_millis_times_100(&self) -> f64 {
        self.nanoseconds as f64 / 10_000.0
    }

    pub fn as_seconds(&self) -> f64 {
        self.nanoseconds as f64 / 1_000_000_000.0
    }
}