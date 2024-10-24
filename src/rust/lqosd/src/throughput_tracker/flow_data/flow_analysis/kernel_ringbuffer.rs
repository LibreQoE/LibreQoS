//! Connects to the "flowbee_events" ring buffer and processes the events.
use crate::throughput_tracker::flow_data::flow_analysis::rtt_types::RttData;
use fxhash::FxHashMap;
use lqos_sys::flowbee_data::FlowbeeKey;
use lqos_utils::unix_time::time_since_boot;
use std::{
    ffi::c_void, net::{IpAddr, Ipv4Addr, Ipv6Addr}, slice, sync::atomic::AtomicU64, time::Duration
};
use tracing::{warn, error};
use zerocopy::FromBytes;
use std::sync::OnceLock;

static EVENT_COUNT: AtomicU64 = AtomicU64::new(0);
static EVENTS_PER_SECOND: AtomicU64 = AtomicU64::new(0);

const BUFFER_SIZE: usize = 1024;

struct RttBuffer {
    index: usize,
    buffer: [[RttData; BUFFER_SIZE]; 2],
    last_seen: u64,
    has_new_data: [bool; 2],
}

impl RttBuffer {
    fn new(reading: u64, direction: u32, last_seen: u64) -> Self {
        let empty = [RttData::from_nanos(0); BUFFER_SIZE];
        let mut filled = [RttData::from_nanos(0); BUFFER_SIZE];
        filled[0] = RttData::from_nanos(reading);

        if direction == 0 {
            Self {
                index: 1,
                buffer: [empty, filled],
                last_seen,
                has_new_data: [false, true],
            }
        } else {
            Self {
                index: 0,
                buffer: [filled, empty],
                last_seen,
                has_new_data: [true, false],
            }
        }
    }

    fn push(&mut self, reading: u64, direction: u32, last_seen: u64) {
        self.buffer[direction as usize][self.index] = RttData::from_nanos(reading);
        self.index = (self.index + 1) % BUFFER_SIZE;
        self.last_seen = last_seen;
        self.has_new_data[direction as usize] = true;
    }

    fn median_new_data(&self, direction: usize) -> RttData {
        if !self.has_new_data[direction] {
            // Reject with no new data
            return RttData::from_nanos(0);
        }
        let mut sorted = self.buffer[direction].iter().filter(|x| x.as_nanos() > 0).collect::<Vec<_>>();
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
    fn new() -> Self {
        let config = lqos_config::load_config().unwrap();
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
                            let ip: Ipv6Addr = split[0].parse().unwrap();
                            ip
                        } else {
                            // It's IPv4
                            mask = split[1].parse().unwrap_or(32);
                            let ip: Ipv4Addr = split[0].parse().unwrap();
                            mask += 96;
                            ip.to_ipv6_mapped()
                        };
                        //println!("{:?} {:?}", ip, mask);

                        let addr = ip_network::IpNetwork::new(ip, mask).unwrap();
                        ignore_subnets.insert(addr, true);
                    } else {
                        error!("Invalid subnet: {}", subnet);
                        continue;                    
                    }

                }
            }
        }

        Self {
            flow_rtt: FxHashMap::default(),
            ignore_subnets,
        }
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
static FLOW_BYTES: crossbeam_queue::SegQueue<[u8; EVENT_SIZE]> = crossbeam_queue::SegQueue::new();

#[derive(Debug)]
enum FlowCommands {
    ExpireRttFlows,
    RttMap(tokio::sync::oneshot::Sender<FxHashMap<FlowbeeKey, [RttData; 2]>>),
}

impl FlowActor {
    pub fn start() -> anyhow::Result<()> {
        let (tx, rx) = crossbeam_channel::bounded::<()>(65536);
        // Placeholder for when you need to read the flow system.
        let (cmd_tx, cmd_rx) = crossbeam_channel::bounded::<FlowCommands>(16);

        // Spawn a task to receive events from the eBPF/kernel
        // receiver
        std::thread::Builder::new()
            .name("FlowActor".to_string())
            .spawn(move || {
                let mut flows = FlowTracker::new();

                use crossbeam_channel::select;

                loop {
                    select! {
                        // A flow command arrives
                        recv(cmd_rx) -> msg => {
                            match msg {
                                Ok(FlowCommands::ExpireRttFlows) => {
                                    if let Ok(now) = time_since_boot() {
                                        let since_boot = Duration::from(now);
                                        let expire = (since_boot - Duration::from_secs(30)).as_nanos() as u64;
                                        flows.flow_rtt.retain(|_, v| v.last_seen > expire);
                                        flows.flow_rtt.shrink_to_fit();
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
                                while let Some(msg) = FLOW_BYTES.pop() {
                                    FlowActor::receive_flow(&mut flows, msg.as_slice());
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

    #[inline(always)]
    fn receive_flow(flows: &mut FlowTracker, message: &[u8]) {
        if let Ok(incoming) = FlowbeeEvent::read_from_bytes(message) {
            EVENT_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
             if let Ok(now) = time_since_boot() {
                let since_boot = Duration::from(now);
                if incoming.rtt == 0 {
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
                let entry = flows.flow_rtt.entry(incoming.key)
                    .or_insert(RttBuffer::new(incoming.rtt, incoming.effective_direction, since_boot.as_nanos() as u64));
                entry.push(incoming.rtt, incoming.effective_direction, since_boot.as_nanos() as u64);
            }
        }
    }
}

#[repr(C)]
#[derive(FromBytes, Debug, Clone, PartialEq, Eq, Hash)]
pub struct FlowbeeEvent {
    key: FlowbeeKey,
    rtt: u64,
    effective_direction: u32,
}

#[no_mangle]
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
        let data_slice: &[u8] = slice::from_raw_parts(data_u8, EVENT_SIZE);
        FLOW_BYTES.push(data_slice.try_into().unwrap());
        if tx.try_send(()).is_err() {
            warn!("Could not submit flow event - buffer full");
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
        }
    } else {
        warn!("Flow command arrived before the actor is ready. Dropping it.");
    }

    let result = tokio::runtime::Runtime::new().unwrap().block_on(rx);
    result.unwrap_or_default()
}

pub fn get_rtt_events_per_second() -> u64 {
    EVENTS_PER_SECOND.swap(0, std::sync::atomic::Ordering::Relaxed)
}
