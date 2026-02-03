//! Connects to the "flowbee_events" ring buffer and processes the events.

use fxhash::FxHashMap;
use lqos_sys::flowbee_data::FlowbeeKey;
use lqos_utils::rtt::{FlowbeeEffectiveDirection, RttBuffer, RttData};
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

//const BUFFER_SIZE: usize = 1024;

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
static FLOW_BYTES: Lazy<crossbeam_queue::ArrayQueue<FlowbeeEvent>> =
    Lazy::new(|| crossbeam_queue::ArrayQueue::new(65536 * 32));

#[derive(Debug)]
enum FlowCommands {
    ExpireRttFlows,
    RttMap(tokio::sync::oneshot::Sender<FxHashMap<FlowbeeKey, RttBuffer>>),
}

impl FlowActor {
    pub fn start() -> anyhow::Result<()> {
        // This is a wakeup channel (not a data transport). Keep it tiny so wakeups coalesce.
        // The actual data lives in `FLOW_BYTES`.
        let (tx, rx) = crossbeam_channel::bounded::<()>(1);
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
                let tick = crossbeam_channel::tick(Duration::from_secs(1));

                fn handle_command(flows: &mut FlowTracker, command: FlowCommands) {
                    match command {
                        FlowCommands::ExpireRttFlows => {
                            if let Ok(now) = time_since_boot() {
                                let since_boot = Duration::from(now);
                                let expire = since_boot
                                    .saturating_sub(Duration::from_secs(30))
                                    .as_nanos() as u64;
                                flows.flow_rtt.retain(|_, v| v.last_seen > expire);
                            }
                        }
                        FlowCommands::RttMap(reply) => {
                            let mut keys_to_clear: Vec<FlowbeeKey> = Vec::new();
                            let result = flows
                                .flow_rtt
                                .iter()
                                .filter_map(|(k, v)| {
                                    let snapshot = v.snapshot_if_new_data()?;
                                    keys_to_clear.push(k.clone());
                                    Some((k.clone(), snapshot))
                                })
                                .collect();

                            // Only clear freshness if the receiver is still alive.
                            // Otherwise, we'll try again on the next request so data isn't lost.
                            if reply.send(result).is_ok() {
                                for key in keys_to_clear {
                                    if let Some(buffer) = flows.flow_rtt.get_mut(&key) {
                                        buffer.clear_freshness();
                                    }
                                }
                            }
                        }
                    }
                }

                fn drain_events(
                    flows: &mut FlowTracker,
                    cmd_rx: &crossbeam_channel::Receiver<FlowCommands>,
                ) {
                    // Drain a bounded batch per iteration so we can interleave command handling
                    // under heavy RTT event load.
                    const MAX_BATCH: usize = 4096;

                    loop {
                        // Prioritize any queued commands between RTT batches.
                        while let Ok(command) = cmd_rx.try_recv() {
                            handle_command(flows, command);
                        }

                        let since_boot_nanos = {
                            let Ok(now) = time_since_boot() else {
                                // Keep the queue intact and try again on the next wake/tick.
                                return;
                            };
                            Duration::from(now).as_nanos() as u64
                        };

                        let mut processed = 0usize;
                        while processed < MAX_BATCH {
                            let Some(msg) = FLOW_BYTES.pop() else { break };
                            FlowActor::receive_flow(flows, msg, since_boot_nanos);
                            processed += 1;
                        }

                        // Queue is empty (or we couldn't pop). We're done.
                        if processed < MAX_BATCH {
                            break;
                        }

                        // If we hit the batch limit, yield and keep draining if needed.
                        if FLOW_BYTES.is_empty() {
                            break;
                        }
                        std::thread::yield_now();
                    }
                }

                loop {
                    select! {
                        // A flow command arrives
                        recv(cmd_rx) -> msg => {
                            match msg {
                                Ok(command) => handle_command(&mut flows, command),
                                Err(e) => error!("Error handling flow actor message: {e:?}"),
                            }
                        }
                        // A flow event arrives
                        recv(rx) -> msg => {
                            if let Ok(_) = msg {
                                drain_events(&mut flows, &cmd_rx);
                            }
                        }
                        // Ensure we wake periodically even under very low event rates
                        // (or if a coalesced wakeup is missed).
                        recv(tick) -> _ => {
                            if !FLOW_BYTES.is_empty() {
                                drain_events(&mut flows, &cmd_rx);
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
    fn receive_flow(flows: &mut FlowTracker, incoming: FlowbeeEvent, since_boot_nanos: u64) {
        EVENT_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
            since_boot_nanos,
        ));
        entry.push(
            incoming.rtt,
            incoming.effective_direction.as_direction(),
            since_boot_nanos,
        );
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
        let Ok(event) = FlowbeeEvent::read_from_bytes(data_slice) else {
            return 0;
        };
        if let Ok(_) = FLOW_BYTES.push(event) {
            // Wake FlowActor, but coalesce wakeups under load.
            let _ = tx.try_send(());
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

pub fn flowbee_rtt_map() -> FxHashMap<FlowbeeKey, RttBuffer> {
    let (tx, rx) = tokio::sync::oneshot::channel::<FxHashMap<FlowbeeKey, RttBuffer>>();
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
