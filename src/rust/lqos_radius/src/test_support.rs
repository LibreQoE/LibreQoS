//! Test helpers for constructing minimal RADIUS packets.

use crate::RadiusCode;
use crate::attribute_type::VENDOR_SPECIFIC;

use crate::packet::{
    MESSAGE_AUTHENTICATOR_ATTRIBUTE_LEN, MESSAGE_AUTHENTICATOR_TYPE,
    MESSAGE_AUTHENTICATOR_VALUE_LEN, RADIUS_ATTRIBUTE_HEADER_LEN, RADIUS_AUTHENTICATOR_LEN,
    RADIUS_HEADER_LEN, RADIUS_MAX_ACCOUNTING_PACKET_LEN, RADIUS_MAX_PACKET_LEN, encode_radius_tlv,
    expected_message_authenticator, expected_request_authenticator, message_authenticator_index,
    parse_packet,
};

pub(crate) const SHARED_SECRET: &[u8] = b"radius-secret";

pub(crate) fn accounting_request_packet(identifier: u8, attributes: &[u8]) -> Vec<u8> {
    radius_packet(RadiusCode::AccountingRequest, identifier, attributes)
}

pub(crate) fn radius_attributes(attributes: &[Vec<u8>]) -> Vec<u8> {
    attributes.concat()
}

pub(crate) fn radius_attribute(kind: u8, value: &[u8]) -> Vec<u8> {
    encode_radius_tlv(kind, value)
}

pub(crate) fn radius_text_attribute(kind: u8, value: &str) -> Vec<u8> {
    radius_attribute(kind, value.as_bytes())
}

pub(crate) fn radius_u32_attribute(kind: u8, value: u32) -> Vec<u8> {
    radius_attribute(kind, &value.to_be_bytes())
}

pub(crate) fn radius_vendor_attribute(vendor_id: u32, vendor_type: u8, value: &[u8]) -> Vec<u8> {
    let subattribute = radius_vendor_subattribute(vendor_type, value);
    radius_vendor_attributes(vendor_id, &[subattribute])
}

pub(crate) fn radius_vendor_attributes(vendor_id: u32, subattributes: &[Vec<u8>]) -> Vec<u8> {
    let vendor_payload_len = subattributes.iter().map(Vec::len).sum::<usize>();
    let mut vendor_value = Vec::with_capacity(4 + vendor_payload_len);
    vendor_value.extend_from_slice(&vendor_id.to_be_bytes());
    for subattribute in subattributes {
        vendor_value.extend_from_slice(subattribute);
    }
    radius_attribute(VENDOR_SPECIFIC, &vendor_value)
}

pub(crate) fn radius_vendor_subattribute(vendor_type: u8, value: &[u8]) -> Vec<u8> {
    radius_attribute(vendor_type, value)
}

pub(crate) fn radius_raw_vendor_attribute(vendor_id: u32, value: &[u8]) -> Vec<u8> {
    let mut vendor_value = Vec::with_capacity(4 + value.len());
    vendor_value.extend_from_slice(&vendor_id.to_be_bytes());
    vendor_value.extend_from_slice(value);
    radius_attribute(VENDOR_SPECIFIC, &vendor_value)
}

pub(crate) fn max_sized_accounting_request_packet(identifier: u8) -> Vec<u8> {
    accounting_request_packet_with_total_len(identifier, RADIUS_MAX_ACCOUNTING_PACKET_LEN)
}

pub(crate) fn accounting_request_packet_with_total_len(
    identifier: u8,
    packet_len: usize,
) -> Vec<u8> {
    assert!(
        (RADIUS_HEADER_LEN + RADIUS_ATTRIBUTE_HEADER_LEN..=RADIUS_MAX_PACKET_LEN)
            .contains(&packet_len)
    );

    let mut remaining_attribute_bytes = packet_len - RADIUS_HEADER_LEN;
    let mut attributes = Vec::with_capacity(remaining_attribute_bytes);
    let mut kind = 1_u8;
    while remaining_attribute_bytes > u8::MAX as usize {
        let attribute_len = if remaining_attribute_bytes - u8::MAX as usize == 1 {
            u8::MAX as usize - 1
        } else {
            u8::MAX as usize
        };
        attributes.push(kind);
        attributes.push(attribute_len as u8);
        attributes.resize(
            attributes.len() + attribute_len - RADIUS_ATTRIBUTE_HEADER_LEN,
            kind,
        );
        remaining_attribute_bytes -= attribute_len;
        kind = kind.wrapping_add(1);
    }

    let final_attribute_len = remaining_attribute_bytes;
    attributes.push(kind);
    attributes.push(final_attribute_len as u8);
    attributes.resize(
        attributes.len() + final_attribute_len - RADIUS_ATTRIBUTE_HEADER_LEN,
        0,
    );

    accounting_request_packet(identifier, &attributes)
}

pub(crate) fn radius_packet(code: RadiusCode, identifier: u8, attributes: &[u8]) -> Vec<u8> {
    let packet_len = RADIUS_HEADER_LEN + attributes.len();
    assert!(packet_len <= RADIUS_MAX_PACKET_LEN);

    let mut packet = Vec::with_capacity(packet_len);
    packet.push(code.as_u8());
    packet.push(identifier);
    packet.extend_from_slice(&(packet_len as u16).to_be_bytes());
    packet.extend_from_slice(&[1_u8; RADIUS_AUTHENTICATOR_LEN]);
    packet.extend_from_slice(attributes);
    packet
}

pub(crate) fn signed_accounting_request_packet(
    identifier: u8,
    attributes: &[u8],
    shared_secret: &[u8],
) -> Vec<u8> {
    signed_radius_packet(
        RadiusCode::AccountingRequest,
        identifier,
        attributes,
        shared_secret,
    )
}

pub(crate) fn signed_radius_packet(
    code: RadiusCode,
    identifier: u8,
    attributes: &[u8],
    shared_secret: &[u8],
) -> Vec<u8> {
    let mut packet = radius_packet(code, identifier, attributes);
    let parsed = parse_packet(&packet).unwrap();
    let request_authenticator = expected_request_authenticator(&parsed, shared_secret);
    packet[4..RADIUS_HEADER_LEN].copy_from_slice(&request_authenticator);
    packet
}

pub(crate) fn accounting_request_packet_with_message_authenticator(
    identifier: u8,
    attributes: &[u8],
    shared_secret: &[u8],
) -> Vec<u8> {
    let mut attributes_with_message_authenticator =
        Vec::with_capacity(attributes.len() + 2 + MESSAGE_AUTHENTICATOR_VALUE_LEN);
    attributes_with_message_authenticator.extend_from_slice(attributes);
    attributes_with_message_authenticator.push(MESSAGE_AUTHENTICATOR_TYPE);
    attributes_with_message_authenticator.push(MESSAGE_AUTHENTICATOR_ATTRIBUTE_LEN as u8);
    attributes_with_message_authenticator
        .extend_from_slice(&[0_u8; MESSAGE_AUTHENTICATOR_VALUE_LEN]);
    let message_authenticator_value_offset = attributes.len() + RADIUS_ATTRIBUTE_HEADER_LEN;

    accounting_request_packet_with_message_authenticator_at(
        identifier,
        &attributes_with_message_authenticator,
        message_authenticator_value_offset,
        shared_secret,
    )
}

pub(crate) fn accounting_request_packet_with_message_authenticator_at(
    identifier: u8,
    attributes_with_message_authenticator: &[u8],
    message_authenticator_value_offset: usize,
    shared_secret: &[u8],
) -> Vec<u8> {
    assert!(
        message_authenticator_value_offset + MESSAGE_AUTHENTICATOR_VALUE_LEN
            <= attributes_with_message_authenticator.len()
    );
    let mut packet = accounting_request_packet(identifier, attributes_with_message_authenticator);
    let value_start = RADIUS_HEADER_LEN + message_authenticator_value_offset;
    let value_end = value_start + MESSAGE_AUTHENTICATOR_VALUE_LEN;
    let parsed = parse_packet(&packet).unwrap();
    let message_authenticator_index = message_authenticator_index(&parsed).unwrap().unwrap();
    let message_authenticator =
        expected_message_authenticator(&parsed, shared_secret, message_authenticator_index);
    packet[value_start..value_end].copy_from_slice(&message_authenticator);
    let parsed = parse_packet(&packet).unwrap();
    let request_authenticator = expected_request_authenticator(&parsed, shared_secret);
    packet[4..RADIUS_HEADER_LEN].copy_from_slice(&request_authenticator);
    packet
}
