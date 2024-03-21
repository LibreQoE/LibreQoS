//! Protocol definitions for Netflow v9 Data.
//! Mostly derived from https://netflow.caligare.com/netflow_v9.htm

use lqos_sys::flowbee_data::FlowbeeKey;
mod field_types;
use field_types::*;

use crate::throughput_tracker::flow_data::FlowbeeLocalData;
pub(crate) mod field_encoder;
pub(crate) mod header;
pub(crate) mod template_ipv4;
pub(crate) mod template_ipv6;

fn add_field(bytes: &mut Vec<u8>, field_type: u16, field_length: u16) {
    bytes.extend_from_slice(field_type.to_be_bytes().as_ref());
    bytes.extend_from_slice(field_length.to_be_bytes().as_ref());
}

pub(crate) fn to_netflow_9(
    key: &FlowbeeKey,
    data: &FlowbeeLocalData,
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

fn ipv4_record(key: &FlowbeeKey, data: &FlowbeeLocalData, direction: usize) -> anyhow::Result<Vec<u8>> {
    let field_bytes = field_encoder::encode_fields_from_template(
        &template_ipv4::FIELDS_IPV4,
        direction,
        key,
        data,
    )?;

    // Build the actual record
    let mut bytes = Vec::new();
    // Add the flowset_id. Template ID is 256
    bytes.extend_from_slice(&(256u16).to_be_bytes());

    // Add the length. Length includes 2 bytes for flowset and 2 bytes for the length field
    // itself. That's odd.
    let padding = (field_bytes.len() + 4) % 4;
    let size = (bytes.len() + field_bytes.len() + padding + 2) as u16;
    bytes.extend_from_slice(&size.to_be_bytes());

    // Add the data itself
    bytes.extend_from_slice(&field_bytes);

    println!("Padding: {}", padding);
    println!("IPv4 data {} = {}", bytes.len(), size);
    println!("Field bytes was: {}", field_bytes.len());

    // Pad to 32-bits
    for _ in 0..padding {
        bytes.push(0);
    }

    Ok(bytes)
}

fn ipv6_record(key: &FlowbeeKey, data: &FlowbeeLocalData, direction: usize) -> anyhow::Result<Vec<u8>> {
    let field_bytes = field_encoder::encode_fields_from_template(
        &template_ipv6::FIELDS_IPV6,
        direction,
        key,
        data,
    )?;

    // Build the actual record
    let mut bytes = Vec::new();
    // Add the flowset_id. Template ID is 257
    bytes.extend_from_slice(&(257u16).to_be_bytes());

    // Add the length. Length includes 2 bytes for flowset and 2 bytes for the length field
    // itself. That's odd.
    let padding = (field_bytes.len() + 4) % 4;
    let size = (bytes.len() + field_bytes.len() + padding + 2) as u16;
    bytes.extend_from_slice(&size.to_be_bytes());

    // Add the data itself
    bytes.extend_from_slice(&field_bytes);

    // Pad to 32-bits
    while bytes.len() % 4 != 0 {
        bytes.push(0);
    }

    Ok(bytes)
}
