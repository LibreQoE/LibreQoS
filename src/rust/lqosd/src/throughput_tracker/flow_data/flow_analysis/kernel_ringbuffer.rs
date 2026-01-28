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
use tracing::{error, warn};
use zerocopy::FromBytes;

static EVENT_COUNT: AtomicU64 = AtomicU64::new(0);
static EVENTS_PER_SECOND: AtomicU64 = AtomicU64::new(0);

const BUFFER_SIZE: usize = 1024;

struct RttBuffer2 {
    last_seen: u64,
    has_new_data: [bool; 2],
    current_bucket: [u32; 27],
    total_bucket: [u32; 27],
}

impl RttBuffer2 {
    pub const fn bucket_nanos(nanos: u64) -> usize {
        match nanos {
            0..5_000_000 => 0,                    // 0–5ms
            5_000_000..10_000_000 => 1,           // 5–10ms
            10_000_000..15_000_000 => 2,          // 10–15ms
            15_000_000..20_000_000 => 3,          // 15–20ms
            20_000_000..25_000_000 => 4,          // 20–25ms
            25_000_000..30_000_000 => 5,          // 25–30ms
            30_000_000..35_000_000 => 6,          // 30–35ms
            35_000_000..40_000_000 => 7,          // 35–40ms
            40_000_000..45_000_000 => 8,          // 40–45ms
            45_000_000..50_000_000 => 9,          // 45–50ms
            50_000_000..60_000_000 => 10,         // 50–60ms
            60_000_000..70_000_000 => 11,         // 60–70ms
            70_000_000..80_000_000 => 12,         // 70–80ms
            80_000_000..90_000_000 => 13,         // 80–90ms
            90_000_000..100_000_000 => 14,        // 90–100ms
            100_000_000..120_000_000 => 15,       // 100–120ms
            120_000_000..140_000_000 => 16,       // 120–140ms
            140_000_000..160_000_000 => 17,       // 140–160ms
            160_000_000..180_000_000 => 18,       // 160–180ms
            180_000_000..200_000_000 => 19,       // 180–200ms
            200_000_000..250_000_000 => 20,       // 200–250ms
            250_000_000..300_000_000 => 21,       // 250–300ms
            300_000_000..400_000_000 => 22,       // 300–400ms
            400_000_000..500_000_000 => 23,       // 400–500ms
            500_000_000..750_000_000 => 24,       // 500–750ms
            750_000_000..1_000_000_000 => 25,     // 750–1000ms
            1_000_000_000.. => 26,                // >1000ms
        }
    }

    pub fn bucket(rtt: RttData) -> usize {
        /*
Dear Codex,

Please make this function return a bucket index for these ranges. If this
function can be constified, that'd be lovely.

0–5
5–10
10–15
15–20
20–25
25–30
30–35
35–40
40–45
45–50

50–60
60–70
70–80
80–90
90–100
100–120
120–140
140–160
160–180
180–200

200–250
250–300
300–400
400–500
500–750
750–1000
>1000

         */
        Self::bucket_nanos(rtt.as_nanos())
    }
}

#[derive(Allocative)]
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
}

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
                                        .map(|(k, v)| (k.clone(), [v.median_new_data(0), v.median_new_data(1)]))
                                        .collect();

                                    // Clear all fresh data labeling
                                    flows.flow_rtt.iter_mut().for_each(|(_, v)| {
                                        v.has_new_data = [false, false];
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
                    incoming.effective_direction,
                    since_boot.as_nanos() as u64,
                ));
                entry.push(
                    incoming.rtt,
                    incoming.effective_direction,
                    since_boot.as_nanos() as u64,
                );
            }
        }
    }
}

#[repr(C)]
#[derive(FromBytes, Debug, Clone, PartialEq, Eq, Hash)]
pub struct FlowbeeEvent {
    key: FlowbeeKey,
    rtt: RttData,
    effective_direction: u32,
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
