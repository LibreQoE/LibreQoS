//! Packet tests for RADIUS accounting framing and authentication.

use super::*;
use crate::attribute_type::ACCT_STATUS_TYPE;
use crate::test_support::{
    SHARED_SECRET, accounting_request_packet, accounting_request_packet_with_message_authenticator,
    accounting_request_packet_with_message_authenticator_at,
    accounting_request_packet_with_total_len, max_sized_accounting_request_packet, radius_packet,
    signed_accounting_request_packet, signed_radius_packet,
};

const ACCOUNTING_START_ATTRIBUTE: [u8; 6] = [ACCT_STATUS_TYPE, 6, 0, 0, 0, 1];
const FIXED_ACCOUNTING_REQUEST: [u8; 26] = [
    4, 7, 0, 26, 234, 109, 208, 193, 96, 89, 23, 174, 213, 177, 203, 9, 123, 217, 127, 22, 40, 6,
    0, 0, 0, 1,
];
const FIXED_ACCOUNTING_RESPONSE: [u8; 20] = [
    5, 7, 0, 20, 35, 106, 121, 65, 4, 237, 106, 10, 15, 20, 216, 122, 222, 158, 144, 160,
];
const FIXED_PROXY_ACCOUNTING_REQUEST: [u8; 33] = [
    4, 9, 0, 33, 96, 121, 47, 229, 56, 17, 23, 167, 140, 77, 87, 100, 57, 14, 218, 128, 40, 6, 0,
    0, 0, 1, 33, 7, 112, 114, 111, 120, 121,
];
const FIXED_PROXY_ACCOUNTING_RESPONSE: [u8; 27] = [
    5, 9, 0, 27, 125, 39, 136, 19, 21, 211, 192, 196, 215, 106, 80, 129, 89, 194, 251, 134, 33, 7,
    112, 114, 111, 120, 121,
];
// Matches FreeRADIUS v3.2.x Accounting-Request handling: HMAC with the request
// vector zeroed, then Accounting MD5 with the Message-Authenticator value present.
const FIXED_MESSAGE_AUTHENTICATOR_REQUEST: [u8; 44] = [
    4, 11, 0, 44, 17, 225, 44, 198, 149, 240, 255, 183, 213, 29, 93, 75, 88, 244, 33, 62, 40, 6, 0,
    0, 0, 1, 80, 18, 190, 213, 77, 117, 156, 25, 155, 188, 192, 113, 189, 123, 238, 165, 229, 7,
];
const FIXED_MESSAGE_AUTHENTICATOR_RESPONSE: [u8; 20] = [
    5, 11, 0, 20, 120, 245, 78, 218, 247, 24, 76, 17, 208, 88, 100, 153, 63, 124, 107, 242,
];
const FIXED_MULTI_PROXY_ACCOUNTING_REQUEST: [u8; 39] = [
    4, 12, 0, 39, 158, 213, 142, 15, 167, 123, 197, 221, 162, 68, 20, 166, 78, 6, 157, 152, 40, 6,
    0, 0, 0, 1, 33, 5, 111, 110, 101, 241, 3, 9, 33, 5, 116, 119, 111,
];
const FIXED_MULTI_PROXY_ACCOUNTING_RESPONSE: [u8; 30] = [
    5, 12, 0, 30, 178, 201, 10, 40, 135, 29, 40, 136, 69, 8, 142, 127, 30, 126, 130, 165, 33, 5,
    111, 110, 101, 33, 5, 116, 119, 111,
];
const RAW_INVALID_PACKET_LENGTH_FIXTURE: [u8; 20] = [
    ACCOUNTING_REQUEST_CODE,
    13,
    0,
    (RADIUS_HEADER_LEN - 1) as u8,
    1,
    1,
    1,
    1,
    1,
    1,
    1,
    1,
    1,
    1,
    1,
    1,
    1,
    1,
    1,
    1,
];
const UNKNOWN_ATTRIBUTE_TYPE: u8 = 241;

#[test]
fn parses_accounting_request_with_attributes() {
    let request =
        handle_accounting_request(&accounting_request_packet(7, &ACCOUNTING_START_ATTRIBUTE))
            .unwrap();
    let packet = request.packet();

    assert_eq!(packet.code(), RadiusCode::AccountingRequest);
    assert_eq!(packet.identifier(), 7);
    assert_eq!(packet.authenticator(), &[1_u8; RADIUS_AUTHENTICATOR_LEN]);
    assert_eq!(packet.attributes().len(), 1);
    assert_eq!(packet.attributes()[0].kind(), ACCT_STATUS_TYPE);
    assert_eq!(packet.attributes()[0].value(), &[0, 0, 0, 1]);
}

#[test]
fn parses_minimal_accounting_request_raw_fixture() {
    let datagram = [
        ACCOUNTING_REQUEST_CODE,
        42,
        0,
        RADIUS_HEADER_LEN as u8,
        0,
        1,
        2,
        3,
        4,
        5,
        6,
        7,
        8,
        9,
        10,
        11,
        12,
        13,
        14,
        15,
    ];
    let parsed = parse_packet(&datagram).unwrap();

    assert_eq!(parsed.code(), RadiusCode::AccountingRequest);
    assert_eq!(parsed.identifier(), 42);
    assert_eq!(
        parsed.authenticator(),
        &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
    );
    assert_eq!(parsed.attributes(), &[]);
}

#[test]
fn rejects_short_packet_header() {
    assert_eq!(
        parse_packet(&[ACCOUNTING_REQUEST_CODE, 1]),
        Err(PacketError::PacketTooShort { actual: 2 })
    );
}

#[test]
fn rejects_declared_packet_length_below_header() {
    assert_eq!(
        parse_packet(&RAW_INVALID_PACKET_LENGTH_FIXTURE),
        Err(PacketError::InvalidPacketLength {
            declared: RADIUS_HEADER_LEN - 1,
        })
    );
}

#[test]
fn rejects_declared_packet_length_above_maximum() {
    assert_eq!(
        parse_packet(&packet_with_declared_len(
            (RADIUS_MAX_PACKET_LEN + 1) as u16
        )),
        Err(PacketError::InvalidPacketLength {
            declared: RADIUS_MAX_PACKET_LEN + 1,
        })
    );
}

#[test]
fn parses_declared_packet_length_at_radius_maximum() {
    let packet = accounting_request_packet_with_total_len(7, RADIUS_MAX_PACKET_LEN);
    let parsed = parse_packet(&packet).unwrap();

    assert_eq!(packet.len(), RADIUS_MAX_PACKET_LEN);
    assert_eq!(parsed.attributes().len(), 16);
    assert_eq!(parsed.attributes()[15].kind(), 16);
    assert_eq!(parsed.attributes()[15].value().len(), 249);
}

#[test]
fn parses_generated_packet_length_that_would_leave_one_byte_chunk() {
    let packet = accounting_request_packet_with_total_len(7, RADIUS_HEADER_LEN + 256);
    let parsed = parse_packet(&packet).unwrap();

    assert_eq!(packet.len(), RADIUS_HEADER_LEN + 256);
    assert_eq!(parsed.attributes().len(), 2);
    assert_eq!(parsed.attributes()[0].value().len(), 252);
    assert_eq!(parsed.attributes()[1].value().len(), 0);
}

#[test]
fn accounting_handler_accepts_declared_packet_length_at_accounting_maximum() {
    let packet = max_sized_accounting_request_packet(7);
    let request = handle_accounting_request(&packet).unwrap();

    assert_eq!(packet.len(), RADIUS_MAX_ACCOUNTING_PACKET_LEN);
    assert_eq!(request.packet().attributes().len(), 16);
    assert_eq!(request.packet().attributes()[15].kind(), 16);
    assert_eq!(request.packet().attributes()[15].value().len(), 248);
}

#[test]
fn accounting_handler_rejects_declared_packet_length_above_accounting_maximum() {
    let packet = accounting_request_packet_with_total_len(7, RADIUS_MAX_PACKET_LEN);

    assert_eq!(
        handle_accounting_request(&packet),
        Err(PacketError::InvalidAccountingPacketLength {
            declared: RADIUS_MAX_PACKET_LEN,
        })
    );
}

#[test]
fn rejects_truncated_declared_packet_length() {
    assert_eq!(
        parse_packet(&packet_with_declared_len((RADIUS_HEADER_LEN + 1) as u16)),
        Err(PacketError::PacketTruncated {
            declared: RADIUS_HEADER_LEN + 1,
            actual: RADIUS_HEADER_LEN,
        })
    );
}

#[test]
fn parses_empty_attribute_value_and_continues() {
    let mut attributes = vec![UNKNOWN_ATTRIBUTE_TYPE, 2];
    attributes.extend_from_slice(&ACCOUNTING_START_ATTRIBUTE);
    let packet = accounting_request_packet(7, &attributes);
    let parsed = parse_packet(&packet).unwrap();

    assert_eq!(parsed.attributes().len(), 2);
    assert_eq!(parsed.attributes()[0].kind(), UNKNOWN_ATTRIBUTE_TYPE);
    assert_eq!(parsed.attributes()[0].value(), &[] as &[u8]);
    assert_eq!(parsed.attributes()[1].kind(), ACCT_STATUS_TYPE);
    assert_eq!(parsed.attributes()[1].value(), &[0, 0, 0, 1]);
}

#[test]
fn rejects_truncated_attribute_header() {
    let packet = accounting_request_packet(7, &[ACCT_STATUS_TYPE]);

    assert_eq!(
        parse_packet(&packet),
        Err(PacketError::AttributeHeaderTruncated {
            offset: RADIUS_HEADER_LEN,
        })
    );
}

#[test]
fn rejects_attribute_length_smaller_than_header() {
    let packet = accounting_request_packet(7, &[ACCT_STATUS_TYPE, 1]);

    assert_eq!(
        parse_packet(&packet),
        Err(PacketError::InvalidAttributeLength {
            offset: RADIUS_HEADER_LEN,
            declared: 1,
        })
    );
}

#[test]
fn rejects_attribute_length_overrun() {
    let mut packet = vec![ACCOUNTING_REQUEST_CODE, 7, 0, 24];
    packet.extend_from_slice(&[1_u8; RADIUS_AUTHENTICATOR_LEN]);
    packet.extend_from_slice(&[ACCT_STATUS_TYPE, 6, 0, 0]);

    assert_eq!(
        parse_packet(&packet),
        Err(PacketError::AttributeTruncated {
            offset: RADIUS_HEADER_LEN,
            declared: 6,
            remaining: 4,
        })
    );
}

#[test]
fn ignores_padding_after_declared_length() {
    let mut packet = accounting_request_packet(7, &[]);
    packet.extend_from_slice(&[ACCT_STATUS_TYPE, 1]);

    let parsed = parse_packet(&packet).unwrap();

    assert_eq!(parsed.attributes(), &[]);
}

#[test]
fn accounting_handler_rejects_non_accounting_code() {
    let packet = radius_packet(RadiusCode::AccessRequest, 7, &ACCOUNTING_START_ATTRIBUTE);

    assert_eq!(
        handle_accounting_request(&packet),
        Err(PacketError::UnsupportedCode { code: 1 })
    );
}

#[test]
fn verifies_fixed_accounting_request_authenticator() {
    let verified = verify_optional(&FIXED_ACCOUNTING_REQUEST).unwrap();

    assert_eq!(verified.packet().identifier(), 7);
    assert!(!verified.has_message_authenticator());
    assert_eq!(verified.packet().attributes()[0].kind(), ACCT_STATUS_TYPE);
    assert_eq!(verified.packet().attributes()[0].value(), &[0, 0, 0, 1]);
}

#[test]
fn rejects_accounting_request_with_wrong_secret() {
    assert_eq!(
        verify_accounting_request(
            &FIXED_ACCOUNTING_REQUEST,
            b"wrong-secret",
            MessageAuthenticatorPolicy::Optional
        ),
        Err(PacketError::InvalidRequestAuthenticator)
    );
}

#[test]
fn rejects_accounting_request_with_modified_attributes() {
    let mut packet =
        signed_accounting_request_packet(7, &ACCOUNTING_START_ATTRIBUTE, SHARED_SECRET);
    let final_attribute_byte = packet.len() - 1;
    packet[final_attribute_byte] ^= 1;

    assert_eq!(
        verify_optional(&packet),
        Err(PacketError::InvalidRequestAuthenticator)
    );
}

#[test]
fn verification_rejects_unsupported_code() {
    let packet = signed_radius_packet(
        RadiusCode::AccessRequest,
        7,
        &ACCOUNTING_START_ATTRIBUTE,
        SHARED_SECRET,
    );

    assert_eq!(
        verify_optional(&packet),
        Err(PacketError::UnsupportedCode { code: 1 })
    );
}

#[test]
fn verification_rejects_truncated_packet_before_authentication() {
    let mut packet =
        signed_accounting_request_packet(7, &ACCOUNTING_START_ATTRIBUTE, SHARED_SECRET);
    packet.truncate(RADIUS_HEADER_LEN + ACCOUNTING_START_ATTRIBUTE.len() - 1);

    assert_eq!(
        verify_optional(&packet),
        Err(PacketError::PacketTruncated {
            declared: RADIUS_HEADER_LEN + ACCOUNTING_START_ATTRIBUTE.len(),
            actual: RADIUS_HEADER_LEN + ACCOUNTING_START_ATTRIBUTE.len() - 1,
        })
    );
}

#[test]
fn verifies_fixed_message_authenticator_when_optional() {
    let verified = verify_optional(&FIXED_MESSAGE_AUTHENTICATOR_REQUEST).unwrap();

    assert_eq!(verified.packet().identifier(), 11);
    assert!(verified.has_message_authenticator());
}

#[test]
fn verifies_fixed_message_authenticator_when_required() {
    let verified = verify_required(&FIXED_MESSAGE_AUTHENTICATOR_REQUEST).unwrap();

    assert_eq!(verified.packet().identifier(), 11);
    assert!(verified.has_message_authenticator());
}

#[test]
fn fixed_message_authenticator_uses_accounting_interop_order() {
    let value_start =
        RADIUS_HEADER_LEN + ACCOUNTING_START_ATTRIBUTE.len() + RADIUS_ATTRIBUTE_HEADER_LEN;
    let value_end = value_start + MESSAGE_AUTHENTICATOR_VALUE_LEN;
    let mut received_message_authenticator = [0_u8; MESSAGE_AUTHENTICATOR_VALUE_LEN];
    received_message_authenticator
        .copy_from_slice(&FIXED_MESSAGE_AUTHENTICATOR_REQUEST[value_start..value_end]);
    let mut received_request_authenticator = [0_u8; RADIUS_AUTHENTICATOR_LEN];
    received_request_authenticator
        .copy_from_slice(&FIXED_MESSAGE_AUTHENTICATOR_REQUEST[4..RADIUS_HEADER_LEN]);

    let mut hmac_input = FIXED_MESSAGE_AUTHENTICATOR_REQUEST.to_vec();
    hmac_input[4..RADIUS_HEADER_LEN].fill(0);
    hmac_input[value_start..value_end].fill(0);
    assert_eq!(
        hmac_md5(SHARED_SECRET, &hmac_input),
        received_message_authenticator
    );

    let mut md5_input = FIXED_MESSAGE_AUTHENTICATOR_REQUEST.to_vec();
    md5_input[4..RADIUS_HEADER_LEN].fill(0);
    md5_input.extend_from_slice(SHARED_SECRET);
    assert_eq!(md5_digest(&md5_input), received_request_authenticator);

    let mut old_md5_input = FIXED_MESSAGE_AUTHENTICATOR_REQUEST.to_vec();
    old_md5_input[4..RADIUS_HEADER_LEN].fill(0);
    old_md5_input[value_start..value_end].fill(0);
    old_md5_input.extend_from_slice(SHARED_SECRET);
    assert_ne!(md5_digest(&old_md5_input), received_request_authenticator);
}

#[test]
fn rejects_message_authenticator_packet_with_invalid_request_authenticator() {
    let mut packet = FIXED_MESSAGE_AUTHENTICATOR_REQUEST;
    packet[4] ^= 1;

    assert_eq!(
        verify_optional(&packet),
        Err(PacketError::InvalidRequestAuthenticator)
    );
}

#[test]
fn verifies_message_authenticator_before_other_attributes() {
    let mut attributes = vec![
        MESSAGE_AUTHENTICATOR_TYPE,
        MESSAGE_AUTHENTICATOR_ATTRIBUTE_LEN as u8,
    ];
    attributes.extend_from_slice(&[0_u8; MESSAGE_AUTHENTICATOR_VALUE_LEN]);
    attributes.extend_from_slice(&ACCOUNTING_START_ATTRIBUTE);
    let packet = accounting_request_packet_with_message_authenticator_at(
        7,
        &attributes,
        RADIUS_ATTRIBUTE_HEADER_LEN,
        SHARED_SECRET,
    );
    let verified = verify_required(&packet).unwrap();

    assert_eq!(verified.packet().attributes().len(), 2);
    assert_eq!(verified.packet().attributes()[1].kind(), ACCT_STATUS_TYPE);
}

#[test]
fn rejects_invalid_message_authenticator() {
    let mut packet = accounting_request_packet_with_message_authenticator(
        7,
        &ACCOUNTING_START_ATTRIBUTE,
        SHARED_SECRET,
    );
    let message_authenticator_value = packet.len() - MESSAGE_AUTHENTICATOR_VALUE_LEN;
    packet[message_authenticator_value] ^= 1;

    assert_eq!(
        verify_optional(&packet),
        Err(PacketError::InvalidMessageAuthenticator)
    );
}

#[test]
fn rejects_modified_accounting_attribute_with_message_authenticator() {
    let mut packet = FIXED_MESSAGE_AUTHENTICATOR_REQUEST;
    let accounting_status_value = RADIUS_HEADER_LEN + RADIUS_ATTRIBUTE_HEADER_LEN + 3;
    packet[accounting_status_value] ^= 1;

    assert_eq!(
        verify_optional(&packet),
        Err(PacketError::InvalidMessageAuthenticator)
    );
}

#[test]
fn rejects_multiple_message_authenticators() {
    let mut attributes = ACCOUNTING_START_ATTRIBUTE.to_vec();
    attributes.extend_from_slice(&[
        MESSAGE_AUTHENTICATOR_TYPE,
        MESSAGE_AUTHENTICATOR_ATTRIBUTE_LEN as u8,
    ]);
    attributes.extend_from_slice(&[0_u8; MESSAGE_AUTHENTICATOR_VALUE_LEN]);
    attributes.extend_from_slice(&[
        MESSAGE_AUTHENTICATOR_TYPE,
        MESSAGE_AUTHENTICATOR_ATTRIBUTE_LEN as u8,
    ]);
    attributes.extend_from_slice(&[0_u8; MESSAGE_AUTHENTICATOR_VALUE_LEN]);
    let packet = signed_accounting_request_packet(7, &attributes, SHARED_SECRET);

    assert_eq!(
        verify_optional(&packet),
        Err(PacketError::MultipleMessageAuthenticators)
    );
}

#[test]
fn rejects_missing_message_authenticator_when_required() {
    let packet = signed_accounting_request_packet(7, &ACCOUNTING_START_ATTRIBUTE, SHARED_SECRET);

    assert_eq!(
        verify_required(&packet),
        Err(PacketError::MissingMessageAuthenticator)
    );
}

#[test]
fn rejects_malformed_message_authenticator_length() {
    let mut attributes = ACCOUNTING_START_ATTRIBUTE.to_vec();
    attributes.extend_from_slice(&[MESSAGE_AUTHENTICATOR_TYPE, 3, 1]);
    let packet = signed_accounting_request_packet(7, &attributes, SHARED_SECRET);

    assert_eq!(
        verify_optional(&packet),
        Err(PacketError::InvalidMessageAuthenticatorLength { actual: 1 })
    );
}

#[test]
fn builds_accounting_response_code_5() {
    let packet = signed_accounting_request_packet(7, &ACCOUNTING_START_ATTRIBUTE, SHARED_SECRET);
    let verified = verify_optional(&packet).unwrap();
    let response = build_accounting_response(&verified, SHARED_SECRET);
    let parsed_response = parse_packet(&response).unwrap();

    assert_eq!(response.len(), RADIUS_HEADER_LEN);
    assert_eq!(response[0], RadiusCode::AccountingResponse.as_u8());
    assert_eq!(u16::from_be_bytes([response[2], response[3]]), 20);
    assert_eq!(parsed_response.code(), RadiusCode::AccountingResponse);
    assert_eq!(parsed_response.identifier(), 7);
    assert_eq!(parsed_response.attributes(), &[]);
}

#[test]
fn builds_fixed_accounting_response_fixture() {
    let verified = verify_optional(&FIXED_ACCOUNTING_REQUEST).unwrap();

    assert_eq!(
        build_accounting_response(&verified, SHARED_SECRET),
        FIXED_ACCOUNTING_RESPONSE
    );
}

#[test]
fn builds_accounting_response_for_message_authenticator_request() {
    let verified = verify_required(&FIXED_MESSAGE_AUTHENTICATOR_REQUEST).unwrap();
    let response = build_accounting_response(&verified, SHARED_SECRET);
    let parsed_response = parse_packet(&response).unwrap();

    assert_eq!(response.len(), RADIUS_HEADER_LEN);
    assert_eq!(response, FIXED_MESSAGE_AUTHENTICATOR_RESPONSE);
    assert_eq!(parsed_response.code(), RadiusCode::AccountingResponse);
    assert_eq!(parsed_response.identifier(), 11);
    assert_eq!(parsed_response.attributes(), &[]);
}

#[test]
fn accounting_response_preserves_proxy_state_attributes() {
    let verified = verify_optional(&FIXED_PROXY_ACCOUNTING_REQUEST).unwrap();
    let response = build_accounting_response(&verified, SHARED_SECRET);
    let parsed_response = parse_packet(&response).unwrap();

    assert_eq!(response, FIXED_PROXY_ACCOUNTING_RESPONSE);
    assert_eq!(parsed_response.attributes().len(), 1);
    assert_eq!(parsed_response.attributes()[0].kind(), PROXY_STATE_TYPE);
    assert_eq!(parsed_response.attributes()[0].value(), b"proxy");
}

#[test]
fn accounting_response_preserves_all_proxy_state_attributes_only() {
    let verified = verify_optional(&FIXED_MULTI_PROXY_ACCOUNTING_REQUEST).unwrap();
    let response = build_accounting_response(&verified, SHARED_SECRET);
    let parsed_response = parse_packet(&response).unwrap();

    assert_eq!(response, FIXED_MULTI_PROXY_ACCOUNTING_RESPONSE);
    assert_eq!(parsed_response.attributes().len(), 2);
    assert_eq!(parsed_response.attributes()[0].kind(), PROXY_STATE_TYPE);
    assert_eq!(parsed_response.attributes()[0].value(), b"one");
    assert_eq!(parsed_response.attributes()[1].kind(), PROXY_STATE_TYPE);
    assert_eq!(parsed_response.attributes()[1].value(), b"two");
}

fn verify_optional(datagram: &[u8]) -> Result<VerifiedAccountingRequest, PacketError> {
    verify_accounting_request(
        datagram,
        SHARED_SECRET,
        MessageAuthenticatorPolicy::Optional,
    )
}

fn verify_required(datagram: &[u8]) -> Result<VerifiedAccountingRequest, PacketError> {
    verify_accounting_request(
        datagram,
        SHARED_SECRET,
        MessageAuthenticatorPolicy::Required,
    )
}

fn packet_with_declared_len(declared_len: u16) -> Vec<u8> {
    let mut packet = vec![ACCOUNTING_REQUEST_CODE, 1];
    packet.extend_from_slice(&declared_len.to_be_bytes());
    packet.extend_from_slice(&[1_u8; RADIUS_AUTHENTICATOR_LEN]);
    packet
}
