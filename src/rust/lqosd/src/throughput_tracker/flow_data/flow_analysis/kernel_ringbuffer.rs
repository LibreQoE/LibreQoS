//! Connects to the "flowbee_events" ring buffer and processes the events.
use crate::throughput_tracker::flow_data::flow_analysis::rtt_types::RttData;
use allocative::Allocative;
use fxhash::FxHashMap;
use lqos_sys::flowbee_data::FlowbeeKey;
use lqos_utils::unix_time::time_since_boot;
use once_cell::sync::Lazy;
use std::sync::OnceLock;
use std::{
    ffi::c_void,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    slice,
    sync::atomic::AtomicU64,
    time::Instant,
    time::Duration,
};
use smallvec::smallvec;
use tracing::{error, warn};
use zerocopy::FromBytes;

static EVENT_COUNT: AtomicU64 = AtomicU64::new(0);
static EVENTS_PER_SECOND: AtomicU64 = AtomicU64::new(0);

const BUFFER_SIZE: usize = 1024;

struct RttBufferBucket {
    current_bucket: [u32; 37],
    total_bucket: [u32; 37],
    current_bucket_start_time_nanos: u64,
    best_rtt: Option<RttData>,
    worst_rtt: Option<RttData>,
    has_new_data: bool,
}

impl Default for RttBufferBucket {
    fn default() -> Self {
        Self {
            current_bucket: [0; 37],
            total_bucket: [0; 37],
            current_bucket_start_time_nanos: 0,
            best_rtt: None,
            worst_rtt: None,
            has_new_data: false,
        }
    }
}

const NS_PER_MS: u64 = 1_000_000;

// Bucket counts
const BUCKET_1MS_MAX: u64 = 10; // 0–10ms
const BUCKET_2MS_MAX: u64 = 20; // 10–20ms

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

enum RttBucket {
    Current,
    Total
}

struct RttBuffer {
    last_seen: u64,
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

    fn clear_freshness(&mut self) {
        // Note: called in the collector system
        self.download_bucket.has_new_data = false;
        self.upload_bucket.has_new_data = false;
    }

    fn new(reading: RttData, direction: FlowbeeEffectiveDirection, last_seen: u64) -> Self {
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

    fn push(&mut self, reading: RttData, direction: FlowbeeEffectiveDirection, last_seen: u64) {
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
        let mut targets: Vec<u32> = percentiles
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

    fn median_new_data(&self, direction: FlowbeeEffectiveDirection) -> RttData {
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

/*#[derive(Allocative)]
struct RttBuffer {
    index: usize,
    buffer: [[RttData; BUFFER_SIZE]; 2],
    last_seen: u64,
    has_new_data: [bool; 2],
}

impl RttBuffer {
    fn new(reading: RttData, direction: u32, last_seen: u64) -> Self {
        let empty = [RttData::from_nanos(0); BUFFER_SIZE];
        let mut filled = [RttData::from_nanos(0); BUFFER_SIZE];
        filled[0] = reading;

        if direction == 0 {
            Self {
                index: 1,
                buffer: [filled, empty],
                last_seen,
                has_new_data: [true, false],
            }
        } else {
            Self {
                index: 1,
                buffer: [empty, filled],
                last_seen,
                has_new_data: [false, true],
            }
        }
    }

    fn push(&mut self, reading: RttData, direction: u32, last_seen: u64) {
        self.buffer[direction as usize][self.index] = reading;
        self.index = (self.index + 1) % BUFFER_SIZE;
        self.last_seen = last_seen;
        self.has_new_data[direction as usize] = true;
    }

    fn median_new_data(&self, direction: usize) -> RttData {
        if !self.has_new_data[direction] {
            // Reject with no new data
            return RttData::from_nanos(0);
        }
        let mut sorted = self.buffer[direction]
            .iter()
            .filter(|x| x.as_nanos() > 0)
            .collect::<Vec<_>>();
        if sorted.is_empty() {
            return RttData::from_nanos(0);
        }
        sorted.sort_unstable();
        let mid = sorted.len() / 2;
        *sorted[mid]
    }
}*/

struct FlowTracker {
    flow_rtt: FxHashMap<FlowbeeKey, RttBuffer>,
    ignore_subnets: ip_network_table::IpNetworkTable<bool>,
}

impl FlowTracker {
    fn new() -> anyhow::Result<Self> {
        let Ok(config) = lqos_config::load_config() else {
            error!("Unable to read configuration. Flow tracker cannot run.");
            anyhow::bail!("Unable to build flow tracker");
        };
        let mut ignore_subnets = ip_network_table::IpNetworkTable::new();
        if let Some(flows) = &config.flows {
            if let Some(subnets) = &flows.do_not_track_subnets {
                // Subnets are in CIDR notation
                for subnet in subnets.iter() {
                    let mut mask;
                    if subnet.contains('/') {
                        let split = subnet.split('/').collect::<Vec<_>>();
                        //println!("{:?}", split);
                        if split.len() != 2 {
                            error!("Invalid subnet: {}", subnet);
                            continue;
                        }
                        let ip = if split[0].contains(":") {
                            // It's IPv6
                            mask = split[1].parse().unwrap_or(128);
                            let ip: Ipv6Addr = split[0].parse()?;
                            ip
                        } else {
                            // It's IPv4
                            mask = split[1].parse().unwrap_or(32);
                            let ip: Ipv4Addr = split[0].parse()?;
                            mask += 96;
                            ip.to_ipv6_mapped()
                        };
                        //println!("{:?} {:?}", ip, mask);

                        let addr = ip_network::IpNetwork::new(ip, mask)?;
                        ignore_subnets.insert(addr, true);
                    } else {
                        error!("Invalid subnet: {}", subnet);
                        continue;
                    }
                }
            }
        }

        Ok(Self {
            flow_rtt: FxHashMap::default(),
            ignore_subnets,
        })
    }
}

/// Provides an actor-model approach to flow tracking storage.
/// The goal is to avoid locking on flow storage, and provide a
/// low-impact means of injecting flow data from the kernel into
/// the tracker as fast as possible (missing as few events as possible).
pub struct FlowActor {}

const EVENT_SIZE: usize = size_of::<FlowbeeEvent>();

static FLOW_BYTES_SENDER: OnceLock<crossbeam_channel::Sender<()>> = OnceLock::new();
static FLOW_COMMAND_SENDER: OnceLock<crossbeam_channel::Sender<FlowCommands>> = OnceLock::new();
static FLOW_BYTES: Lazy<crossbeam_queue::ArrayQueue<[u8; EVENT_SIZE]>> =
    Lazy::new(|| crossbeam_queue::ArrayQueue::new(65536 * 32));

#[derive(Debug)]
enum FlowCommands {
    ExpireRttFlows,
    RttMap(tokio::sync::oneshot::Sender<FxHashMap<FlowbeeKey, [RttData; 2]>>),
}

impl FlowActor {
    pub fn start() -> anyhow::Result<()> {
        let (tx, rx) = crossbeam_channel::bounded::<()>(131_072);
        // Placeholder for when you need to read the flow system.
        let (cmd_tx, cmd_rx) = crossbeam_channel::bounded::<FlowCommands>(16);

        // Spawn a task to receive events from the eBPF/kernel
        // receiver
        std::thread::Builder::new()
            .name("FlowActor".to_string())
            .spawn(move || {
                let Ok(mut flows) = FlowTracker::new() else {
                    error!("Flow tracker cannot start");
                    return;
                };

                use crossbeam_channel::select;

                loop {
                    select! {
                        // A flow command arrives
                        recv(cmd_rx) -> msg => {
                            match msg {
                                Ok(FlowCommands::ExpireRttFlows) => {
                                    if let Ok(now) = time_since_boot() {
                                        let since_boot = Duration::from(now);
                                        let expire = since_boot.saturating_sub(Duration::from_secs(30)).as_nanos() as u64;
                                        flows.flow_rtt.retain(|_, v| v.last_seen > expire);
                                    }
                                }
                                Ok(FlowCommands::RttMap(reply)) => {
                                    let result = flows.flow_rtt.iter()
                                        .map(|(k, v)| (k.clone(), [v.median_new_data(FlowbeeEffectiveDirection::Download), v.median_new_data(FlowbeeEffectiveDirection::Upload)]))
                                        .collect();

                                    // Clear all fresh data labeling
                                    flows.flow_rtt.iter_mut().for_each(|(_, v)| {
                                        v.clear_freshness();
                                    });
                                    let _ = reply.send(result);
                                }
                                _ => error!("Error handling flow actor message: {msg:?}"),
                            }
                        }
                        // A flow event arrives
                        recv(rx) -> msg => {
                            if let Ok(_) = msg {
                                // Drain a bounded batch to avoid starving command handling
                                // under heavy RTT event load.
                                const MAX_BATCH: usize = 4096;
                                let mut processed = 0usize;
                                while processed < MAX_BATCH {
                                    let Some(msg) = FLOW_BYTES.pop() else { break };
                                    FlowActor::receive_flow(&mut flows, msg.as_slice());
                                    processed += 1;
                                }
                                if processed == MAX_BATCH {
                                    std::thread::yield_now();
                                }
                            }
                        }
                    }
                }
            })?;

        // Store the submission sender
        if let Err(e) = FLOW_BYTES_SENDER.set(tx) {
            error!("Unable to setup flow tracking channel. {e:?}");
            anyhow::bail!("Unable to setup flow tracking channel.");
        }

        // Store the command sender
        if let Err(e) = FLOW_COMMAND_SENDER.set(cmd_tx) {
            error!("Unable to setup flow tracking command channel. {e:?}");
            anyhow::bail!("Unable to setup flow tracking command channel.");
        }

        Ok(())
    }

    #[inline]
    fn receive_flow(flows: &mut FlowTracker, message: &[u8]) {
        if let Ok(incoming) = FlowbeeEvent::read_from_bytes(message) {
            EVENT_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if let Ok(now) = time_since_boot() {
                let since_boot = Duration::from(now);
                if incoming.rtt.as_nanos() == 0 {
                    return;
                }

                // Check if it should be ignored
                let ip = incoming.key.remote_ip.as_ip();
                let ip = match ip {
                    IpAddr::V4(ip) => ip.to_ipv6_mapped(),
                    IpAddr::V6(ip) => ip,
                };
                if flows.ignore_subnets.longest_match(ip).is_some() {
                    return;
                }

                // Insert it
                let entry = flows.flow_rtt.entry(incoming.key).or_insert(RttBuffer::new(
                    incoming.rtt,
                    incoming.effective_direction.as_direction(),
                    since_boot.as_nanos() as u64,
                ));
                entry.push(
                    incoming.rtt,
                    incoming.effective_direction.as_direction(),
                    since_boot.as_nanos() as u64,
                );
            }
        }
    }
}

#[repr(C)]
#[derive(FromBytes, Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct FlowbeeDirectionRaw {
    direction: u32,
}

impl FlowbeeDirectionRaw {
    fn as_direction(&self) -> FlowbeeEffectiveDirection {
        match self.direction {
            0 => FlowbeeEffectiveDirection::Download,
            1 => FlowbeeEffectiveDirection::Upload,
            _ => panic!("Invalid direction"),
        }
    }
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlowbeeEffectiveDirection {
    Download = 0,
    Upload = 1
}

#[repr(C)]
#[derive(FromBytes, Debug, Clone, PartialEq, Eq, Hash)]
pub struct FlowbeeEvent {
    key: FlowbeeKey,
    rtt: RttData,
    effective_direction: FlowbeeDirectionRaw,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn flowbee_handle_events(
    _ctx: *mut c_void,
    data: *mut c_void,
    data_size: usize,
) -> i32 {
    if let Some(tx) = FLOW_BYTES_SENDER.get() {
        // Validate the buffer size
        if data_size < EVENT_SIZE {
            warn!("Flow ringbuffer data is too small. Dropping it.");
            return 0;
        }

        // Copy the bytes (to free the ringbuffer slot)
        let data_u8 = data as *const u8;
        let data_slice: &[u8] = unsafe { slice::from_raw_parts(data_u8, EVENT_SIZE) };
        let Ok(data_slice) = data_slice.try_into() else {
            return 0;
        };
        if let Ok(_) = FLOW_BYTES.push(data_slice) {
            if tx.try_send(()).is_err() {
                warn!("Could not submit flow event - buffer full");
            }
        }
    } else {
        warn!("Flow ringbuffer data arrived before the actor is ready. Dropping it.");
        return 0;
    }
    0
}

pub fn get_flowbee_event_count_and_reset() -> u64 {
    let count = EVENT_COUNT.swap(0, std::sync::atomic::Ordering::Relaxed);
    EVENTS_PER_SECOND.store(count, std::sync::atomic::Ordering::Relaxed);
    count
}

pub fn expire_rtt_flows() {
    if let Some(tx) = FLOW_COMMAND_SENDER.get() {
        if tx.try_send(FlowCommands::ExpireRttFlows).is_err() {
            warn!("Could not submit flow command - buffer full");
        }
    } else {
        warn!("Flow command arrived before the actor is ready. Dropping it.");
    }
}

pub fn flowbee_rtt_map() -> FxHashMap<FlowbeeKey, [RttData; 2]> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    if let Some(cmd_tx) = FLOW_COMMAND_SENDER.get() {
        if cmd_tx.try_send(FlowCommands::RttMap(tx)).is_err() {
            warn!("Could not submit flow command - buffer full");
            return FxHashMap::default();
        }
    } else {
        warn!("Flow command arrived before the actor is ready. Dropping it.");
        return FxHashMap::default();
    }

    // Avoid blocking indefinitely if the FlowActor is busy (e.g. under very high RTT
    // event rates). Worst case, return an empty map and let the UI continue updating.
    let deadline = Instant::now() + Duration::from_millis(250);
    let mut rx = rx;
    loop {
        match rx.try_recv() {
            Ok(result) => return result,
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                if Instant::now() >= deadline {
                    warn!("Timed out waiting for RTT map from flow actor");
                    return FxHashMap::default();
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                warn!("Failed to receive RTT map from flow actor - channel closed");
                return FxHashMap::default();
            }
        }
    }
}

pub fn get_rtt_events_per_second() -> u64 {
    EVENTS_PER_SECOND.swap(0, std::sync::atomic::Ordering::Relaxed)
}
