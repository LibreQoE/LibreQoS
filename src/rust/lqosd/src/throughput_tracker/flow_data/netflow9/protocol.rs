//! Protocol definitions for Netflow v9 Data.

use std::net::IpAddr;

use lqos_sys::flowbee_data::{FlowbeeData, FlowbeeKey};
use lqos_utils::unix_time::time_since_boot;
use nix::sys::time::TimeValLike;

pub(crate) struct Netflow9Header {
    pub(crate) version: u16,
    pub(crate) count: u16,
    pub(crate) sys_uptime: u32,
    pub(crate) unix_secs: u32,
    pub(crate) package_sequence: u32,
    pub(crate) source_id: u32,
}

impl Netflow9Header {
    /// Create a new Netflow 9 header
    pub(crate) fn new(flow_sequence: u32) -> Self {
        let uptime = time_since_boot().unwrap();

        Self {
            version: (9u16).to_be(),
            count: (2u16).to_be(),
            sys_uptime: (uptime.num_milliseconds() as u32).to_be(),
            unix_secs: (uptime.num_seconds() as u32).to_be(),
            package_sequence: flow_sequence,
            source_id: 0,
        }
    }
}

fn add_field(bytes: &mut Vec<u8>, field_type: u16, field_length: u16) {
    bytes.extend_from_slice(field_type.to_be_bytes().as_ref());
    bytes.extend_from_slice(field_length.to_be_bytes().as_ref());
}

pub fn template_data_ipv4(sequence: u32) -> Vec<u8> {
    const FIELDS: [(u16, u16); 8] = [
        (1, 4),  // IN_BYTES
        (2, 4),  // IN_PKTS
        (4, 1),  // PROTOCOL
        (7, 4),  // L4_SRC_PORT
        (8, 4),  // IPV4_SRC_ADDR
        (11, 4), // L4_DST_PORT
        (12, 4), // IPV4_DST_ADDR
        (15, 1), // TOS
    ];

    // Build the header
    let mut bytes = Vec::new();

    // Add the flowset_id, id is zero. (See https://netflow.caligare.com/netflow_v9.htm)
    // 16
    bytes.push(0);
    bytes.push(0);

    // Add the length of the flowset, 4 bytes
    const LENGTH: u16 = 4; // TODO: Fixme
    bytes.extend_from_slice(LENGTH.to_be_bytes().as_ref());

    // Add the TemplateID. We're going to use 256 for IPv4.
    const TEMPLATE_ID: u16 = 256;
    bytes.extend_from_slice(TEMPLATE_ID.to_be_bytes().as_ref());

    // Add the number of fields in the template
    const FIELD_COUNT: u16 = FIELDS.len() as u16;
    bytes.extend_from_slice(FIELD_COUNT.to_be_bytes().as_ref());

    for (field_type, field_length) in FIELDS.iter() {
        add_field(&mut bytes, *field_type, *field_length);
    }

    bytes
}

pub fn template_data_ipv6(sequence: u32) -> Vec<u8> {
    const FIELDS: [(u16, u16); 8] = [
        (1, 4),   // IN_BYTES
        (2, 4),   // IN_PKTS
        (4, 1),   // PROTOCOL
        (7, 4),   // L4_SRC_PORT
        (27, 16), // IPV6_SRC_ADDR
        (11, 4),  // L4_DST_PORT
        (28, 16), // IPV6_DST_ADDR
        (15, 1),  // TOS
    ];

    // Build the header
    let mut bytes = Vec::new();

    // Add the flowset_id, id is zero. (See https://netflow.caligare.com/netflow_v9.htm)
    // 16
    bytes.push(0);
    bytes.push(0);

    // Add the length of the flowset, 4 bytes
    const LENGTH: u16 = 4; // TODO: Fixme
    bytes.extend_from_slice(LENGTH.to_be_bytes().as_ref());

    // Add the TemplateID. We're going to use 257 for IPv6.
    const TEMPLATE_ID: u16 = 257;
    bytes.extend_from_slice(TEMPLATE_ID.to_be_bytes().as_ref());

    // Add the number of fields in the template
    const FIELD_COUNT: u16 = FIELDS.len() as u16;
    bytes.extend_from_slice(FIELD_COUNT.to_be_bytes().as_ref());

    for (field_type, field_length) in FIELDS.iter() {
        add_field(&mut bytes, *field_type, *field_length);
    }

    bytes
}

pub(crate) fn to_netflow_9(
    key: &FlowbeeKey,
    data: &FlowbeeData,
) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
    if key.local_ip.is_v4() && key.remote_ip.is_v4() {
        // Return IPv4 records
        Ok((ipv4_record(key, data, 0)?, ipv4_record(key, data, 1)?))
    } else if (!key.local_ip.is_v4()) && (!key.remote_ip.is_v4()) {
        // Return IPv6 records
        Ok((ipv6_record(key, data, 0)?, ipv6_record(key, data, 1)?))
    } else {
        anyhow::bail!("Mixing IPv4 and IPv6 is not supported");
    }
}

fn ipv4_record(key: &FlowbeeKey, data: &FlowbeeData, direction: usize) -> anyhow::Result<Vec<u8>> {
    // Configure IP directions
    let local = key.local_ip.as_ip();
    let remote = key.remote_ip.as_ip();
    if let (IpAddr::V4(local), IpAddr::V4(remote)) = (local, remote) {
        let src_ip = u32::from_ne_bytes(local.octets());
        let dst_ip = u32::from_ne_bytes(remote.octets());

        // Build the field values
        let mut field_bytes: Vec<u8> = Vec::new();

        // Bytes Sent
        field_bytes.extend_from_slice(&data.bytes_sent[direction].to_be_bytes());

        // Packet Sent
        field_bytes.extend_from_slice(&data.packets_sent[direction].to_be_bytes());

        // Add the protocol
        field_bytes.push(key.ip_protocol);

        // Add the source port
        field_bytes.extend_from_slice(&key.src_port.to_be_bytes());

        // Add the source address
        if direction == 0 {
            field_bytes.extend_from_slice(&src_ip.to_be_bytes());
        } else {
            field_bytes.extend_from_slice(&dst_ip.to_be_bytes());
        }

        // Add the destination port
        field_bytes.extend_from_slice(&key.dst_port.to_be_bytes());

        // Add the destination address
        if direction == 0 {
            field_bytes.extend_from_slice(&dst_ip.to_be_bytes());
        } else {
            field_bytes.extend_from_slice(&src_ip.to_be_bytes());
        }

        // Add the TOS
        field_bytes.push(0);

        // Build the actual record
        let mut bytes = Vec::new();
        // Add the flowset_id. Template ID is 256
        bytes.extend_from_slice(&(256u16).to_be_bytes());

        // Add the length. Length includes 2 bytes for flowset and 2 bytes for the length field
        // itself. That's odd.
        bytes.extend_from_slice(&((field_bytes.len() as u16 + 4).to_be_bytes()));

        Ok(bytes)
    } else {
        anyhow::bail!("IPv6 data in an IPv4 function was a bad idea");
    }
}

fn ipv6_record(key: &FlowbeeKey, data: &FlowbeeData, direction: usize) -> anyhow::Result<Vec<u8>> {
    // Configure IP directions
    let local = key.local_ip.as_ip();
    let remote = key.remote_ip.as_ip();
    if let (IpAddr::V6(local), IpAddr::V6(remote)) = (local, remote) {
        let src_ip = local.octets();
        let dst_ip = remote.octets();

        // Build the field values
        let mut field_bytes: Vec<u8> = Vec::new();

        // Bytes Sent
        field_bytes.extend_from_slice(&data.bytes_sent[direction].to_be_bytes());

        // Packet Sent
        field_bytes.extend_from_slice(&data.packets_sent[direction].to_be_bytes());

        // Add the protocol
        field_bytes.push(key.ip_protocol);

        // Add the source port
        field_bytes.extend_from_slice(&key.src_port.to_be_bytes());

        // Add the source address
        if direction == 0 {
            field_bytes.extend_from_slice(&src_ip);
        } else {
            field_bytes.extend_from_slice(&dst_ip);
        }

        // Add the destination port
        field_bytes.extend_from_slice(&key.dst_port.to_be_bytes());

        // Add the destination address
        if direction == 0 {
            field_bytes.extend_from_slice(&dst_ip);
        } else {
            field_bytes.extend_from_slice(&src_ip);
        }

        // Add the TOS
        field_bytes.push(0);

        // Build the actual record
        let mut bytes = Vec::new();
        // Add the flowset_id. Template ID is 257
        bytes.extend_from_slice(&(257u16).to_be_bytes());

        // Add the length. Length includes 2 bytes for flowset and 2 bytes for the length field
        // itself. That's odd.
        bytes.extend_from_slice(&((field_bytes.len() as u16 + 4).to_be_bytes()));

        Ok(bytes)
    } else {
        anyhow::bail!("IPv4 data in an IPv6 function was a bad idea");
    }
}