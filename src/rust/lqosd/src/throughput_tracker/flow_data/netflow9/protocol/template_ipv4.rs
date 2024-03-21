use crate::throughput_tracker::flow_data::netflow9::protocol::*;

pub(crate) const FIELDS_IPV4: [(u16, u16); 8] = [
    IN_BYTES,
    IN_PKTS,
    PROTOCOL,
    L4_SRC_PORT,
    IPV4_SRC_ADDR,
    L4_DST_PORT,
    IPV4_DST_ADDR,
    DST_TOS,
];

pub fn template_data_ipv4() -> Vec<u8> {
    // Build the header
    let mut bytes = Vec::new();

    // Add the flowset_id, id is zero. (See https://netflow.caligare.com/netflow_v9.htm)
    // 16
    bytes.push(0);
    bytes.push(0);

    // Add the length of the flowset, 4 bytes
    const LENGTH: u16 = 8 + (FIELDS_IPV4.len() * 4) as u16; // TODO: Fixme
    bytes.extend_from_slice(LENGTH.to_be_bytes().as_ref());

    // Add the TemplateID. We're going to use 256 for IPv4.
    const TEMPLATE_ID: u16 = 256;
    bytes.extend_from_slice(TEMPLATE_ID.to_be_bytes().as_ref());

    // Add the number of fields in the template
    const FIELD_COUNT: u16 = FIELDS_IPV4.len() as u16;
    bytes.extend_from_slice(FIELD_COUNT.to_be_bytes().as_ref());

    for (field_type, field_length) in FIELDS_IPV4.iter() {
        add_field(&mut bytes, *field_type, *field_length);
    }

    println!("Templatev4 Size {} = {}", bytes.len(), 8 + (FIELDS_IPV4.len() * 2));

    bytes
}
