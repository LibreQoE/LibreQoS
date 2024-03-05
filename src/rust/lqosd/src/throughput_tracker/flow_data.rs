use lqos_sys::flowbee_data::{FlowbeeData, FlowbeeKey};
use lqos_utils::unix_time::time_since_boot;
use nix::sys::time::TimeValLike;
use once_cell::sync::Lazy;
use std::{net::{IpAddr, UdpSocket}, sync::{mpsc::{channel, Sender}, Mutex}};

pub static ALL_FLOWS: Lazy<Mutex<Vec<(FlowbeeKey, FlowbeeData)>>> =
    Lazy::new(|| Mutex::new(Vec::with_capacity(128_000)));

// Creates the netflow tracker and returns the sender
pub fn setup_netflow_tracker() -> Sender<(FlowbeeKey, FlowbeeData)> {
    let (tx, rx) = channel::<(FlowbeeKey, FlowbeeData)>();
    let config = lqos_config::load_config().unwrap();

    std::thread::spawn(move || {
        log::info!("Starting the network flow tracker back-end");

        // Build the endpoints list
        let mut endpoints: Vec<Box<dyn FlowbeeRecipient>> = Vec::new();
        if let Some(flow_config) = config.flows {
            if let (Some(ip), Some(port), Some(version)) = (flow_config.netflow_ip, flow_config.netflow_port, flow_config.netflow_version)
            {
                log::info!("Setting up netflow target: {ip}:{port}, version: {version}");
                let target = format!("{ip}:{port}", ip = ip, port = port);
                match version {
                    5 => {
                        let endpoint = Netflow5::new(target).unwrap();
                        endpoints.push(Box::new(endpoint));
                        log::info!("Netflow 5 endpoint added");
                    }
                    _ => log::error!("Unsupported netflow version: {version}"),
                }
            }
        
        }

        // Send to all endpoints upon receipt
        while let Ok((key, value)) = rx.recv() {
            endpoints.iter_mut().for_each(|f| f.send(key.clone(), value.clone()));
        }
        log::info!("Network flow tracker back-end has stopped")
    });

    tx
}

trait FlowbeeRecipient {
    fn send(&mut self, key: FlowbeeKey, data: FlowbeeData);
}

struct Netflow5 {
    socket: UdpSocket,
    sequence: u32,
    target: String,
}

impl Netflow5 {
    fn new(target: String) -> anyhow::Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:12212")?;
        Ok(Self { socket, sequence: 0, target })
    }
}

impl FlowbeeRecipient for Netflow5 {
    fn send(&mut self, key: FlowbeeKey, data: FlowbeeData) {
        if let Ok((packet1, packet2)) = to_netflow_5(&key, &data) {
            let header = Netflow5Header::new(self.sequence);
            let header_bytes = unsafe { std::slice::from_raw_parts(&header as *const _ as *const u8, std::mem::size_of::<Netflow5Header>()) };
            let packet1_bytes = unsafe { std::slice::from_raw_parts(&packet1 as *const _ as *const u8, std::mem::size_of::<Netflow5Record>()) };
            let packet2_bytes = unsafe { std::slice::from_raw_parts(&packet2 as *const _ as *const u8, std::mem::size_of::<Netflow5Record>()) };
            let mut buffer = Vec::with_capacity(header_bytes.len() + packet1_bytes.len() + packet2_bytes.len());
            buffer.extend_from_slice(header_bytes);
            buffer.extend_from_slice(packet1_bytes);
            buffer.extend_from_slice(packet2_bytes);

            log::debug!("Sending netflow packet to {target}", target = self.target);
            self.socket.send_to(&buffer, &self.target).unwrap();

            self.sequence = self.sequence.wrapping_add(2);
        }
    }
}

#[repr(C)]
struct Netflow5Header {
    version: u16,
    count: u16,
    sys_uptime: u32,
    unix_secs: u32,
    unix_nsecs: u32,
    flow_sequence: u32,
    engine_type: u8,
    engine_id: u8,
    sampling_interval: u16,
}

impl Netflow5Header {
    fn new(flow_sequence: u32) -> Self {
        let uptime = time_since_boot().unwrap();

        Self {
            version: 5,
            count: 2,
            sys_uptime: uptime.num_milliseconds() as u32,
            unix_secs: uptime.num_seconds() as u32,
            unix_nsecs: 0,
            flow_sequence,
            engine_type: 0,
            engine_id: 0,
            sampling_interval: 0,
        }
    }

}

#[repr(C)]
struct Netflow5Record {
    src_addr: u32,
    dst_addr: u32,
    next_hop: u32,
    input: u16,
    output: u16,
    d_pkts: u32,
    d_octets: u32,
    first: u32,
    last: u32,
    src_port: u16,
    dst_port: u16,
    pad1: u8,
    tcp_flags: u8,
    prot: u8,
    tos: u8,
    src_as: u16,
    dst_as: u16,
    src_mask: u8,
    dst_mask: u8,
    pad2: u16,
}

fn to_netflow_5(key: &FlowbeeKey, data: &FlowbeeData) -> anyhow::Result<(Netflow5Record, Netflow5Record)> {
    // TODO: Detect overflow
    let local = key.local_ip.as_ip();
    let remote = key.remote_ip.as_ip();
    if let (IpAddr::V4(local), IpAddr::V4(remote)) = (local, remote) {
        let src_ip = u32::from_ne_bytes(local.octets());
        let dst_ip = u32::from_ne_bytes(remote.octets());
        // Convert d_pkts to network order
        let d_pkts = (data.packets_sent[0] as u32).to_be();
        let d_octets = (data.bytes_sent[0] as u32).to_be();
        let d_pkts2 = (data.packets_sent[1] as u32).to_be();
        let d_octets2 = (data.bytes_sent[1] as u32).to_be();

        let record = Netflow5Record {
            src_addr: src_ip,
            dst_addr: dst_ip,
            next_hop: 0,
            input: 0,
            output: 1,
            d_pkts,
            d_octets,
            first: data.start_time as u32, // Convert to milliseconds
            last: data.last_seen as u32, // Convert to milliseconds
            src_port: key.src_port.to_be(),
            dst_port: key.dst_port.to_be(),
            pad1: 0,
            tcp_flags: 0,
            prot: key.ip_protocol.to_be(),
            tos: 0,
            src_as: 0,
            dst_as: 0,
            src_mask: 0,
            dst_mask: 0,
            pad2: 0,
        };

        let record2 = Netflow5Record {
            src_addr: dst_ip,
            dst_addr: src_ip,
            next_hop: 0,
            input: 1,
            output: 0,
            d_pkts: d_pkts2,
            d_octets: d_octets2,
            first: data.start_time as u32, // Convert to milliseconds
            last: data.last_seen as u32, // Convert to milliseconds
            src_port: key.dst_port.to_be(),
            dst_port: key.src_port.to_be(),
            pad1: 0,
            tcp_flags: 0,
            prot: key.ip_protocol.to_be(),
            tos: 0,
            src_as: 0,
            dst_as: 0,
            src_mask: 0,
            dst_mask: 0,
            pad2: 0,
        };

        Ok((record, record2))
    } else {
        Err(anyhow::anyhow!("Only IPv4 is supported"))
    }
}