use smallvec::smallvec;
use crate::throughput_tracker::flow_data::flow_analysis::FlowbeeEffectiveDirection;
use crate::throughput_tracker::flow_data::RttData;

struct RttBufferBucket {
    current_bucket: [u32; 38],
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

const NS_PER_MS: u64 = 1_000_000;

// Bucket counts

// Offsets
const OFFSET_1MS: usize = 0;
const OFFSET_2MS: usize = OFFSET_1MS + 10; // 10 buckets
const OFFSET_5MS: usize = OFFSET_2MS + 5;  // 5 buckets


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
            0  => 1 * NS_PER_MS,
            1  => 2 * NS_PER_MS,
            2  => 3 * NS_PER_MS,
            3  => 4 * NS_PER_MS,
            4  => 5 * NS_PER_MS,
            5  => 6 * NS_PER_MS,
            6  => 7 * NS_PER_MS,
            7  => 8 * NS_PER_MS,
            8  => 9 * NS_PER_MS,
            9  => 10 * NS_PER_MS,

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
            _  => 1_000 * NS_PER_MS,
        }
    }
}

pub enum RttBucket {
    Current,
    Total
}

pub struct RttBuffer {
    pub(crate) last_seen: u64,
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

    pub(crate) fn clear_freshness(&mut self) {
        // Note: called in the collector system
        self.download_bucket.has_new_data = false;
        self.upload_bucket.has_new_data = false;
    }

    pub(crate) fn new(reading: RttData, direction: FlowbeeEffectiveDirection, last_seen: u64) -> Self {
        let mut entry = Self {
            last_seen,
            download_bucket: RttBufferBucket::default(),
            upload_bucket: RttBufferBucket::default(),
        };
        let target_bucket = entry.pick_bucket_mut(direction);
        let bucket_idx = RttBufferBucket::bucket(reading);
        target_bucket.current_bucket[bucket_idx] += 1; // Safe because we know it was zero previously.
        target_bucket.total_bucket[bucket_idx] += 1;
        target_bucket.current_bucket_start_time_nanos = last_seen;
        target_bucket.best_rtt = Some(reading);
        target_bucket.worst_rtt = Some(reading);
        target_bucket.has_new_data = true;
        entry
    }

    const BUCKET_TIME_NANOS: u64 = 30_000_000_000; // 30 seconds

    pub(crate) fn push(&mut self, reading: RttData, direction: FlowbeeEffectiveDirection, last_seen: u64) {
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
        target_bucket.current_bucket[bucket_idx] = target_bucket.current_bucket[bucket_idx].saturating_add(1);
        target_bucket.total_bucket[bucket_idx] = target_bucket.total_bucket[bucket_idx].saturating_add(1);
        target_bucket.has_new_data = true;
        if let Some(other_max) = target_bucket.worst_rtt {
            target_bucket.worst_rtt = Some(RttData::from_nanos(u64::max(other_max.as_nanos(), reading.as_nanos())));
        } else {
            target_bucket.worst_rtt = Some(reading);
        }
        if let Some(other_min) = target_bucket.best_rtt {
            target_bucket.best_rtt = Some(RttData::from_nanos(u64::min(other_min.as_nanos(), reading.as_nanos())));
        } else {
            target_bucket.best_rtt = Some(reading);
        }
        target_bucket.has_new_data = true; // Note that this is reset on READ
    }

    const MIN_SAMPLES: u32 = 5;

    fn percentiles_from_bucket(&self, scope: RttBucket, direction: FlowbeeEffectiveDirection, percentiles: &[u8]) -> Option<smallvec::SmallVec<[RttData; 3]>> {
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
        // We assume percentiles are in ascending order
        let targets: Vec<u32> = percentiles
            .iter()
            .map(|p| {
                // ceil(p * total / 100)
                ((*p as u32 * total) + 99) / 100
            })
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

        // All percentiles should be filled; if not, something went wrong
        if results.iter().any(|r| r.is_none()) {
            return None;
        }

        Some(results.into_iter().map(|r| r.unwrap()).collect())
    }

    pub(crate) fn median_new_data(&self, direction: FlowbeeEffectiveDirection) -> RttData {
        // Note that this function is kinda sucky, but it's deliberately maintaining
        // the contract - warts and all - of its predecessor. Planned for deprecation
        // later.
        // 0 as a sentinel was a bad idea.
        let target = self.pick_bucket(direction);
        if !target.has_new_data {
            return RttData::from_nanos(0);
        }
        let Some(median) = self.percentiles_from_bucket(
            RttBucket::Current, direction, &[50]
        ) else {
            return RttData::from_nanos(0);
        };
        median[0]
    }
}