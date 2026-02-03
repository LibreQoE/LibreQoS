use allocative_derive::Allocative;
use serde::{Deserialize, Serialize, Serializer};
use smallvec::smallvec;

use super::{FlowbeeEffectiveDirection, RttData};

fn serialize_u32_array_38<S>(value: &[u32; 38], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    value.as_slice().serialize(serializer)
}

#[derive(Clone, Debug, Serialize, Allocative)]
struct RttBufferBucket {
    #[serde(serialize_with = "serialize_u32_array_38")]
    current_bucket: [u32; 38],
    #[serde(serialize_with = "serialize_u32_array_38")]
    total_bucket: [u32; 38],
    current_bucket_start_time_nanos: u64,
    best_rtt: Option<RttData>,
    worst_rtt: Option<RttData>,
    has_new_data: bool,
}

impl Default for RttBufferBucket {
    fn default() -> Self {
        Self {
            current_bucket: [0; 38],
            total_bucket: [0; 38],
            current_bucket_start_time_nanos: 0,
            best_rtt: None,
            worst_rtt: None,
            has_new_data: false,
        }
    }
}

fn accumulate_bucket(dst: &mut RttBufferBucket, src: &RttBufferBucket) {
    for (dst, src) in dst.current_bucket.iter_mut().zip(src.current_bucket.iter()) {
        *dst = dst.saturating_add(*src);
    }
    for (dst, src) in dst.total_bucket.iter_mut().zip(src.total_bucket.iter()) {
        *dst = dst.saturating_add(*src);
    }

    dst.has_new_data |= src.has_new_data;

    dst.best_rtt = match (dst.best_rtt, src.best_rtt) {
        (Some(a), Some(b)) => Some(std::cmp::min(a, b)),
        (None, Some(b)) => Some(b),
        (Some(a), None) => Some(a),
        (None, None) => None,
    };
    dst.worst_rtt = match (dst.worst_rtt, src.worst_rtt) {
        (Some(a), Some(b)) => Some(std::cmp::max(a, b)),
        (None, Some(b)) => Some(b),
        (Some(a), None) => Some(a),
        (None, None) => None,
    };

    dst.current_bucket_start_time_nanos = match (
        dst.current_bucket_start_time_nanos,
        src.current_bucket_start_time_nanos,
    ) {
        (0, s) => s,
        (d, 0) => d,
        (d, s) => u64::min(d, s),
    };
}

const NS_PER_MS: u64 = 1_000_000;

// Offsets
const OFFSET_1MS: usize = 0;
const OFFSET_2MS: usize = OFFSET_1MS + 10; // 10 buckets
const OFFSET_5MS: usize = OFFSET_2MS + 5; // 5 buckets

impl RttBufferBucket {
    #[inline(always)]
    pub const fn bucket(rtt: RttData) -> usize {
        let ns = rtt.as_nanos();
        let ms = ns / NS_PER_MS;

        match ms {
            // 0–10 ms: 1 ms buckets
            0..=9 => OFFSET_1MS + ms as usize,

            // 10–20 ms: 2 ms buckets
            10..=19 => OFFSET_2MS + ((ms - 10) / 2) as usize,

            // 20–25 ms: 5 ms bucket
            20..=24 => OFFSET_5MS + 0,

            // 25–30 ms
            25..=29 => OFFSET_5MS + 1,

            // 30–35 ms
            30..=34 => OFFSET_5MS + 2,

            // 35–40 ms
            35..=39 => OFFSET_5MS + 3,

            // 40–45 ms
            40..=44 => OFFSET_5MS + 4,

            // 45–50 ms
            45..=49 => OFFSET_5MS + 5,

            // then widen progressively, same as before
            50..=59 => OFFSET_5MS + 6,
            60..=69 => OFFSET_5MS + 7,
            70..=79 => OFFSET_5MS + 8,
            80..=89 => OFFSET_5MS + 9,
            90..=99 => OFFSET_5MS + 10,
            100..=119 => OFFSET_5MS + 11,
            120..=139 => OFFSET_5MS + 12,
            140..=159 => OFFSET_5MS + 13,
            160..=179 => OFFSET_5MS + 14,
            180..=199 => OFFSET_5MS + 15,
            200..=249 => OFFSET_5MS + 16,
            250..=299 => OFFSET_5MS + 17,
            300..=399 => OFFSET_5MS + 18,
            400..=499 => OFFSET_5MS + 19,
            500..=749 => OFFSET_5MS + 20,
            750..=999 => OFFSET_5MS + 21,
            _ => OFFSET_5MS + 22,
        }
    }

    #[inline(always)]
    pub const fn bucket_upper_bound_nanos(idx: usize) -> u64 {
        match idx {
            // 1 ms buckets
            0 => 1 * NS_PER_MS,
            1 => 2 * NS_PER_MS,
            2 => 3 * NS_PER_MS,
            3 => 4 * NS_PER_MS,
            4 => 5 * NS_PER_MS,
            5 => 6 * NS_PER_MS,
            6 => 7 * NS_PER_MS,
            7 => 8 * NS_PER_MS,
            8 => 9 * NS_PER_MS,
            9 => 10 * NS_PER_MS,

            // 2 ms buckets
            10 => 12 * NS_PER_MS,
            11 => 14 * NS_PER_MS,
            12 => 16 * NS_PER_MS,
            13 => 18 * NS_PER_MS,
            14 => 20 * NS_PER_MS,

            // widen progressively
            15 => 25 * NS_PER_MS,
            16 => 30 * NS_PER_MS,
            17 => 35 * NS_PER_MS,
            18 => 40 * NS_PER_MS,
            19 => 45 * NS_PER_MS,
            20 => 50 * NS_PER_MS,
            21 => 60 * NS_PER_MS,
            22 => 70 * NS_PER_MS,
            23 => 80 * NS_PER_MS,
            24 => 90 * NS_PER_MS,
            25 => 100 * NS_PER_MS,
            26 => 120 * NS_PER_MS,
            27 => 140 * NS_PER_MS,
            28 => 160 * NS_PER_MS,
            29 => 180 * NS_PER_MS,
            30 => 200 * NS_PER_MS,
            31 => 250 * NS_PER_MS,
            32 => 300 * NS_PER_MS,
            33 => 400 * NS_PER_MS,
            34 => 500 * NS_PER_MS,
            35 => 750 * NS_PER_MS,
            36 => 1_000 * NS_PER_MS,
            _ => 1_000 * NS_PER_MS,
        }
    }
}

/// Which RTT bucket to query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RttBucket {
    /// Current (time-windowed) bucket.
    Current,
    /// Total (lifetime) bucket.
    Total,
}

/// A per-flow or aggregated RTT histogram (download + upload).
#[derive(Clone, Debug, Serialize, Allocative, Default)]
pub struct RttBuffer {
    /// Last-seen timestamp in nanoseconds since boot.
    pub last_seen: u64,
    download_bucket: RttBufferBucket,
    upload_bucket: RttBufferBucket,
}

impl RttBuffer {
    fn pick_bucket_mut(&mut self, direction: FlowbeeEffectiveDirection) -> &mut RttBufferBucket {
        match direction {
            FlowbeeEffectiveDirection::Download => &mut self.download_bucket,
            FlowbeeEffectiveDirection::Upload => &mut self.upload_bucket,
        }
    }

    fn pick_bucket(&self, direction: FlowbeeEffectiveDirection) -> &RttBufferBucket {
        match direction {
            FlowbeeEffectiveDirection::Download => &self.download_bucket,
            FlowbeeEffectiveDirection::Upload => &self.upload_bucket,
        }
    }

    /// Reset this buffer to its default (empty) state.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Accumulate another RTT buffer into this one (saturating bucket counts).
    pub fn accumulate(&mut self, other: &Self) {
        self.last_seen = u64::max(self.last_seen, other.last_seen);
        accumulate_bucket(&mut self.download_bucket, &other.download_bucket);
        accumulate_bucket(&mut self.upload_bucket, &other.upload_bucket);
    }

    /// Accumulate another RTT buffer into this one (saturating bucket counts), but only for one
    /// direction.
    pub fn accumulate_direction(&mut self, other: &Self, direction: FlowbeeEffectiveDirection) {
        self.last_seen = u64::max(self.last_seen, other.last_seen);
        match direction {
            FlowbeeEffectiveDirection::Download => {
                accumulate_bucket(&mut self.download_bucket, &other.download_bucket);
            }
            FlowbeeEffectiveDirection::Upload => {
                accumulate_bucket(&mut self.upload_bucket, &other.upload_bucket);
            }
        }
    }

    /// Clear the per-direction "fresh data" flags.
    ///
    /// This does not clear any histogram counts; it only marks the buffer as having no
    /// newly-observed RTT samples since the last time freshness was cleared.
    pub fn clear_freshness(&mut self) {
        self.download_bucket.has_new_data = false;
        self.upload_bucket.has_new_data = false;
    }

    /// Returns `true` if either direction has received new RTT samples since the last
    /// `clear_freshness()`.
    pub fn has_new_data(&self) -> bool {
        self.download_bucket.has_new_data || self.upload_bucket.has_new_data
    }

    /// Clone and return the buffer if it contains fresh data, otherwise return `None`.
    ///
    /// This is a convenience for "snapshot and ship" code paths: if the caller sends the cloned
    /// snapshot elsewhere, the original can later be marked via `clear_freshness()`.
    pub fn snapshot_if_new_data(&self) -> Option<Self> {
        if self.has_new_data() {
            Some(self.clone())
        } else {
            None
        }
    }

    /// Merge in "fresh" directions from an incoming buffer.
    ///
    /// For each direction, if the incoming bucket is marked as having new data, it replaces the
    /// corresponding bucket in `self`. If the incoming direction has no new data, the existing
    /// bucket is kept.
    pub fn merge_fresh_from(&mut self, incoming: Self) {
        let Self {
            last_seen,
            download_bucket,
            upload_bucket,
        } = incoming;
        self.last_seen = last_seen;

        if download_bucket.has_new_data {
            self.download_bucket = download_bucket;
        }
        if upload_bucket.has_new_data {
            self.upload_bucket = upload_bucket;
        }
    }

    /// Create a new buffer seeded with a single RTT reading.
    pub fn new(
        reading: RttData,
        direction: FlowbeeEffectiveDirection,
        last_seen: u64,
    ) -> Self {
        let mut entry = Self {
            last_seen,
            download_bucket: RttBufferBucket::default(),
            upload_bucket: RttBufferBucket::default(),
        };
        let target_bucket = entry.pick_bucket_mut(direction);
        let bucket_idx = RttBufferBucket::bucket(reading);
        target_bucket.current_bucket[bucket_idx] += 1;
        target_bucket.total_bucket[bucket_idx] += 1;
        target_bucket.current_bucket_start_time_nanos = last_seen;
        target_bucket.best_rtt = Some(reading);
        target_bucket.worst_rtt = Some(reading);
        target_bucket.has_new_data = true;
        entry
    }

    const BUCKET_TIME_NANOS: u64 = 10_000_000_000; // 10 seconds

    /// Push one RTT reading into the histogram.
    ///
    /// - Updates both `RttBucket::Current` (windowed) and `RttBucket::Total` (lifetime) buckets.
    /// - The current bucket is time-windowed (10s) based on `last_seen` and is cleared/rotated when
    ///   the window elapses.
    /// - Bucket counts saturate on overflow.
    pub fn push(
        &mut self,
        reading: RttData,
        direction: FlowbeeEffectiveDirection,
        last_seen: u64,
    ) {
        self.last_seen = last_seen;
        let target_bucket = self.pick_bucket_mut(direction);

        if target_bucket.current_bucket_start_time_nanos == 0 {
            target_bucket.current_bucket_start_time_nanos = last_seen;
        }
        let elapsed = last_seen.saturating_sub(target_bucket.current_bucket_start_time_nanos);
        if elapsed > Self::BUCKET_TIME_NANOS {
            target_bucket.current_bucket_start_time_nanos = last_seen;
            target_bucket.current_bucket.fill(0);
        }

        let bucket_idx = RttBufferBucket::bucket(reading);
        target_bucket.current_bucket[bucket_idx] = target_bucket.current_bucket[bucket_idx]
            .saturating_add(1);
        target_bucket.total_bucket[bucket_idx] = target_bucket.total_bucket[bucket_idx]
            .saturating_add(1);
        target_bucket.has_new_data = true;

        if let Some(other_max) = target_bucket.worst_rtt {
            target_bucket.worst_rtt = Some(RttData::from_nanos(u64::max(
                other_max.as_nanos(),
                reading.as_nanos(),
            )));
        } else {
            target_bucket.worst_rtt = Some(reading);
        }
        if let Some(other_min) = target_bucket.best_rtt {
            target_bucket.best_rtt = Some(RttData::from_nanos(u64::min(
                other_min.as_nanos(),
                reading.as_nanos(),
            )));
        } else {
            target_bucket.best_rtt = Some(reading);
        }
    }

    const MIN_SAMPLES: u32 = 2;

    fn percentiles_from_bucket(
        &self,
        scope: RttBucket,
        direction: FlowbeeEffectiveDirection,
        percentiles: &[u8],
    ) -> Option<smallvec::SmallVec<[RttData; 3]>> {
        let target = self.pick_bucket(direction);
        let buckets = match scope {
            RttBucket::Current => &target.current_bucket,
            RttBucket::Total => &target.total_bucket,
        };

        let total: u32 = buckets.iter().sum();
        if total < Self::MIN_SAMPLES {
            return None;
        }

        // Precompute rank targets (ceil(p/100 * total))
        let targets: Vec<u32> = percentiles
            .iter()
            .map(|p| ((*p as u32 * total) + 99) / 100)
            .collect();

        let mut results: smallvec::SmallVec<[Option<RttData>; 3]> = smallvec![None; percentiles.len()];
        let mut cumulative: u32 = 0;
        let mut next_idx = 0;

        for (bucket_idx, count) in buckets.iter().enumerate() {
            if *count == 0 {
                continue;
            }

            cumulative += count;
            while next_idx < targets.len() && cumulative >= targets[next_idx] {
                let rtt_ns = RttBufferBucket::bucket_upper_bound_nanos(bucket_idx);
                results[next_idx] = Some(RttData::from_nanos(rtt_ns));
                next_idx += 1;
            }

            if next_idx == targets.len() {
                break;
            }
        }

        if results.iter().any(|r| r.is_none()) {
            return None;
        }

        Some(results.into_iter().map(|r| r.unwrap()).collect())
    }

    /// Return the median RTT from the current window if fresh data is present.
    ///
    /// If there is no fresh data (or too few samples), this returns a zero RTT.
    ///
    /// Prefer using `percentile()`/`percentiles()` and explicit freshness handling instead.
    pub fn median_new_data(&self, direction: FlowbeeEffectiveDirection) -> RttData {
        let target = self.pick_bucket(direction);
        if !target.has_new_data {
            return RttData::from_nanos(0);
        }
        let Some(median) = self.percentiles_from_bucket(RttBucket::Current, direction, &[50]) else {
            return RttData::from_nanos(0);
        };
        median[0]
    }

    /// Returns the sample count in the selected histogram scope for a given direction.
    pub fn sample_count(&self, scope: RttBucket, direction: FlowbeeEffectiveDirection) -> u32 {
        let target = self.pick_bucket(direction);
        let buckets = match scope {
            RttBucket::Current => &target.current_bucket,
            RttBucket::Total => &target.total_bucket,
        };
        buckets.iter().sum()
    }

    /// Return one percentile (e.g. p95) as an RTT value (bucket upper bound).
    pub fn percentile(
        &self,
        scope: RttBucket,
        direction: FlowbeeEffectiveDirection,
        percentile: u8,
    ) -> Option<RttData> {
        self.percentiles_from_bucket(scope, direction, &[percentile])
            .map(|v| v[0])
    }

    /// Return multiple percentiles in ascending order.
    pub fn percentiles(
        &self,
        scope: RttBucket,
        direction: FlowbeeEffectiveDirection,
        percentiles: &[u8],
    ) -> Option<smallvec::SmallVec<[RttData; 3]>> {
        self.percentiles_from_bucket(scope, direction, percentiles)
    }
}

#[cfg(test)]
mod tests {
    use super::{FlowbeeEffectiveDirection, RttBuffer, RttBucket, RttData};

    #[test]
    fn accumulate_direction_only_affects_selected_direction() {
        let mut source = RttBuffer::default();
        source.push(
            RttData::from_nanos(1_000_000),
            FlowbeeEffectiveDirection::Download,
            1,
        );
        source.push(
            RttData::from_nanos(2_000_000),
            FlowbeeEffectiveDirection::Download,
            1,
        );
        source.push(
            RttData::from_nanos(3_000_000),
            FlowbeeEffectiveDirection::Upload,
            1,
        );
        source.push(
            RttData::from_nanos(4_000_000),
            FlowbeeEffectiveDirection::Upload,
            1,
        );

        let mut agg = RttBuffer::default();
        agg.accumulate_direction(&source, FlowbeeEffectiveDirection::Download);

        assert_eq!(
            agg.sample_count(RttBucket::Total, FlowbeeEffectiveDirection::Download),
            2
        );
        assert_eq!(
            agg.sample_count(RttBucket::Total, FlowbeeEffectiveDirection::Upload),
            0
        );
        assert_eq!(
            agg.sample_count(RttBucket::Current, FlowbeeEffectiveDirection::Download),
            2
        );
        assert_eq!(
            agg.sample_count(RttBucket::Current, FlowbeeEffectiveDirection::Upload),
            0
        );
    }

    #[test]
    fn accumulate_direction_can_be_applied_twice() {
        let mut source = RttBuffer::default();
        source.push(
            RttData::from_nanos(1_000_000),
            FlowbeeEffectiveDirection::Download,
            1,
        );
        source.push(
            RttData::from_nanos(2_000_000),
            FlowbeeEffectiveDirection::Upload,
            1,
        );

        let mut agg = RttBuffer::default();
        agg.accumulate_direction(&source, FlowbeeEffectiveDirection::Download);
        agg.accumulate_direction(&source, FlowbeeEffectiveDirection::Upload);

        assert_eq!(
            agg.sample_count(RttBucket::Total, FlowbeeEffectiveDirection::Download),
            1
        );
        assert_eq!(
            agg.sample_count(RttBucket::Total, FlowbeeEffectiveDirection::Upload),
            1
        );
    }
}
