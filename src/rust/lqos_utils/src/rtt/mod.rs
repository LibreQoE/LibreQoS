//! RTT (round-trip time) types and helpers.
//!
//! These types are shared between crates (e.g. `lqosd` and `lqos_config`) so
//! RTT aggregation can be performed consistently across the stack.

mod rtt_buffer;
mod rtt_data;

pub use rtt_buffer::{RttBucket, RttBuffer};
pub use rtt_data::RttData;

/// Effective direction of a flow/RTT sample from the shaped device perspective.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlowbeeEffectiveDirection {
    /// Download direction (to the subscriber).
    Download = 0,
    /// Upload direction (from the subscriber).
    Upload = 1,
}

/// Bucket width of the bridge wire's `[u64; 50]` RTT histogram, in
/// milliseconds: the bridge's `latency_bucket` maps
/// `latency_us / 10_000`, i.e. 10 ms per bucket.
pub const RTT_HISTOGRAM_BUCKET_MS: f64 = 10.0;

/// The p90 of a bucketed RTT histogram, in milliseconds (the lower bound of
/// the bucket in which the 90th-percentile rank falls). `None` for an empty
/// histogram.
///
/// K2 slice-2 729b: this is the pure compute used for StormGuard's passive
/// RTT when bridged — the same `latency_bucket` bucketing the bridge wire
/// uses makes it a compute-identity with the wire histogram.
pub fn histogram_p90_ms(buckets: &[u64]) -> Option<f64> {
    let total: u64 = buckets.iter().copied().sum();
    if total == 0 {
        return None;
    }
    // The first bucket whose cumulative count reaches the 90th-percentile
    // rank (ceil of 0.9 * total).
    let target = ((total as f64) * 0.9).ceil() as u64;
    let mut cumulative = 0u64;
    for (index, &count) in buckets.iter().enumerate() {
        cumulative = cumulative.saturating_add(count);
        if cumulative >= target {
            return Some(index as f64 * RTT_HISTOGRAM_BUCKET_MS);
        }
    }
    Some((buckets.len().saturating_sub(1)) as f64 * RTT_HISTOGRAM_BUCKET_MS)
}

#[cfg(test)]
mod histogram_tests {
    use super::*;

    #[test]
    fn empty_histogram_has_no_p90() {
        assert_eq!(histogram_p90_ms(&[0u64; 50]), None);
    }

    #[test]
    fn p90_lands_in_the_rank_bucket() {
        // 100 samples: 80 in bucket 1 (10-20ms), 20 in bucket 5 (50-60ms).
        // Rank 90 falls in bucket 5.
        let mut buckets = [0u64; 50];
        buckets[1] = 80;
        buckets[5] = 20;
        assert_eq!(histogram_p90_ms(&buckets), Some(50.0));
        // 95 in bucket 1, 5 in bucket 8: rank 90 falls in bucket 1.
        let mut buckets = [0u64; 50];
        buckets[1] = 95;
        buckets[8] = 5;
        assert_eq!(histogram_p90_ms(&buckets), Some(10.0));
    }
}
