use std::net::IpAddr;
use lqos_sys::flowbee_data::FlowbeeKey;
use crate::throughput_tracker::flow_data::FlowbeeLocalData;
use super::field_types::*;

pub(crate) fn encode_fields_from_template(template: &[(u16, u16)], direction: usize, key: &FlowbeeKey, data: &FlowbeeLocalData) -> anyhow::Result<Vec<u8>> {
    let src_port = if direction == 0 { key.src_port } else { key.dst_port };
    let dst_port = if direction == 0 { key.dst_port } else { key.src_port };

    let total_size: u16 = template.iter().map(|(_, size)| size).sum();
    let mut result = Vec::with_capacity(total_size as usize);
    for (field_type, field_length) in template.iter() {
        match (*field_type, *field_length) {
            IN_BYTES => encode_u64(data.bytes_sent[direction], &mut result),
            IN_PKTS => encode_u64(data.packets_sent[direction], &mut result),
            PROTOCOL => result.push(key.ip_protocol),
            L4_SRC_PORT => encode_u16(src_port, &mut result),
            L4_DST_PORT => encode_u16(dst_port, &mut result),
            DST_TOS => result.push(data.tos),
            IPV4_SRC_ADDR => encode_ipv4(0, key, &mut result)?,
            IPV4_DST_ADDR => encode_ipv4(1, key, &mut result)?,
            IPV6_SRC_ADDR => encode_ipv6(0, key, &mut result)?,
            IPV6_DST_ADDR => encode_ipv6(1, key, &mut result)?,
            _ => anyhow::bail!("Don't know how to encode field type {} yet", field_type),
        }
    }
    Ok(result)
}

fn encode_u64(value: u64, target: &mut Vec<u8>) {
    target.extend_from_slice(&value.to_be_bytes());
}

fn encode_u16(value: u16, target: &mut Vec<u8>) {
    target.extend_from_slice(&value.to_be_bytes());
}

fn encode_ipv4(direction: usize, key: &FlowbeeKey, target: &mut Vec<u8>) -> anyhow::Result<()> {
    let local = key.local_ip.as_ip();
    let remote = key.remote_ip.as_ip();
    if let (IpAddr::V4(local), IpAddr::V4(remote)) = (local, remote) {
        let src_ip = u32::from_ne_bytes(local.octets());
        let dst_ip = u32::from_ne_bytes(remote.octets());
        if direction == 0 {
            target.extend_from_slice(&src_ip.to_be_bytes());
        } else {
            target.extend_from_slice(&dst_ip.to_be_bytes());
        }
    } else {
        anyhow::bail!("Expected IPv4 addresses, got {:?}", (local, remote));
    }
    Ok(())
}

fn encode_ipv6(direction: usize, key: &FlowbeeKey, target: &mut Vec<u8>) -> anyhow::Result<()> {
    let local = key.local_ip.as_ip();
    let remote = key.remote_ip.as_ip();
    if let (IpAddr::V6(local), IpAddr::V6(remote)) = (local, remote) {
        let src_ip = local.octets();
        let dst_ip = remote.octets();
        if direction == 0 {
            target.extend_from_slice(&src_ip);
        } else {
            target.extend_from_slice(&dst_ip);
        }
    } else {
        anyhow::bail!("Expected IPv6 addresses, got {:?}", (local, remote));
    }
    Ok(())
}