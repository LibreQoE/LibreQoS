//! Connects to the flows.h "flowbee_events" ring buffer and processes the events.
use crate::throughput_tracker::flow_data::flow_analysis::rtt_types::RttData;
use fxhash::FxHashMap;
use lqos_sys::flowbee_data::FlowbeeKey;
use lqos_utils::unix_time::time_since_boot;
use once_cell::sync::Lazy;
use std::{
    ffi::c_void, net::{IpAddr, Ipv4Addr, Ipv6Addr}, slice, sync::{atomic::AtomicU64, Mutex}, time::Duration
};
use zerocopy::FromBytes;

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
                        println!("{:?}", split);
                        if split.len() != 2 {
                            log::error!("Invalid subnet: {}", subnet);
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
                        println!("{:?} {:?}", ip, mask);

                        let addr = ip_network::IpNetwork::new(ip, mask).unwrap();
                        ignore_subnets.insert(addr, true);
                    } else {
                        log::error!("Invalid subnet: {}", subnet);
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

static FLOW_RTT: Lazy<Mutex<FlowTracker>> =
    Lazy::new(|| Mutex::new(FlowTracker::new()));

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
    const EVENT_SIZE: usize = std::mem::size_of::<FlowbeeEvent>();
    if data_size < EVENT_SIZE {
        log::warn!("Warning: incoming data too small in Flowbee buffer");
        return 0;
    }

    let data_u8 = data as *const u8;
    let data_slice: &[u8] = slice::from_raw_parts(data_u8, EVENT_SIZE);
    if let Some(incoming) = FlowbeeEvent::read_from(data_slice) {
        EVENT_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        if let Ok(now) = time_since_boot() {
            let since_boot = Duration::from(now);
            if incoming.rtt == 0 {
                return 0;
            }
            let mut lock = FLOW_RTT.lock().unwrap();
            // Check if it should be ignored
            let ip = incoming.key.remote_ip.as_ip();
            let ip = match ip {
                IpAddr::V4(ip) => {
                    ip.to_ipv6_mapped()
                }
                IpAddr::V6(ip) => {
                    ip
                }
            };

            if lock.ignore_subnets.longest_match(ip).is_some() {
                return 0;
            }

            // Insert it
            if let Some(entry) = lock.flow_rtt.get_mut(&incoming.key) {
                entry.push(
                    incoming.rtt,
                    incoming.effective_direction,
                    since_boot.as_nanos() as u64,
                );
            } else {
                lock.flow_rtt.insert(
                    incoming.key,
                    RttBuffer::new(
                        incoming.rtt,
                        incoming.effective_direction,
                        since_boot.as_nanos() as u64,
                    ),
                );
            }
        }
    } else {
        log::error!("Failed to decode Flowbee Event");
    }

    0
}

pub fn get_flowbee_event_count_and_reset() -> u64 {
    let count = EVENT_COUNT.swap(0, std::sync::atomic::Ordering::Relaxed);
    EVENTS_PER_SECOND.store(count, std::sync::atomic::Ordering::Relaxed);
    count
}

pub fn expire_rtt_flows() {
    if let Ok(now) = time_since_boot() {
        let since_boot = Duration::from(now);
        let expire = (since_boot - Duration::from_secs(30)).as_nanos() as u64;
        let mut lock = FLOW_RTT.lock().unwrap();
        lock.flow_rtt.retain(|_, v| v.last_seen > expire);
    }
}

pub fn flowbee_rtt_map() -> FxHashMap<FlowbeeKey, [RttData; 2]> {
    let mut lock = FLOW_RTT.lock().unwrap();
    let result = lock.flow_rtt.iter()
        .map(|(k, v)| (k.clone(), [v.median_new_data(0), v.median_new_data(1)]))
        .collect();

    // Clear all fresh data labeling
    lock.flow_rtt.iter_mut().for_each(|(_, v)| {
        v.has_new_data = [false, false];
    });

    result
}

pub fn get_rtt_events_per_second() -> u64 {
    EVENTS_PER_SECOND.swap(0, std::sync::atomic::Ordering::Relaxed)
}
