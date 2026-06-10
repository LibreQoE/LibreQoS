//! Tests for typed RADIUS Accounting-Request event extraction.

use super::*;
use crate::packet::MESSAGE_AUTHENTICATOR_TYPE;
use crate::test_support::{
    SHARED_SECRET, accounting_request_packet, radius_attribute, radius_attributes,
    radius_raw_vendor_attribute, radius_text_attribute, radius_u32_attribute,
    radius_vendor_attribute, radius_vendor_attributes, radius_vendor_subattribute,
    signed_accounting_request_packet,
};
use crate::{
    MessageAuthenticatorPolicy, RadiusCode, handle_accounting_request, verify_accounting_request,
};

const RAW_ACCOUNTING_START_FIXTURE: [u8; 26] = [
    4,
    51,
    0,
    26,
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
    ACCT_STATUS_TYPE,
    6,
    0,
    0,
    0,
    1,
];
const RAW_ACCOUNTING_INTERIM_FIXTURE: [u8; 26] = [
    4,
    52,
    0,
    26,
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
    ACCT_STATUS_TYPE,
    6,
    0,
    0,
    0,
    3,
];
const RAW_ACCOUNTING_STOP_FIXTURE: [u8; 26] = [
    4,
    53,
    0,
    26,
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
    ACCT_STATUS_TYPE,
    6,
    0,
    0,
    0,
    2,
];
const RAW_UNKNOWN_VENDOR_IPV6_FIXTURE: [u8; 66] = [
    4,
    54,
    0,
    66,
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
    241,
    5,
    b's',
    b't',
    b'd',
    26,
    12,
    0,
    0,
    48,
    57,
    b'v',
    b'e',
    b'n',
    b'd',
    b'o',
    b'r',
    NAS_IPV6_ADDRESS,
    18,
    0x20,
    0x01,
    0x0d,
    0xb8,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    1,
    FRAMED_IPV6_PREFIX,
    11,
    0,
    56,
    0x20,
    0x01,
    0x0d,
    0xb8,
    0,
    0x10,
    0,
];

#[test]
fn extracts_acct_status_type_variants_and_unknown_values() {
    let cases = [
        (1, AcctStatusType::Start),
        (2, AcctStatusType::Stop),
        (3, AcctStatusType::InterimUpdate),
        (7, AcctStatusType::AccountingOn),
        (8, AcctStatusType::AccountingOff),
        (99, AcctStatusType::Unknown(99)),
    ];

    for (raw, expected) in cases {
        let request = request_with_attributes(&[radius_u32_attribute(ACCT_STATUS_TYPE, raw)]);
        let event = AccountingEvent::from_request(&request);

        assert_eq!(event.status_type, Some(expected));
        assert_eq!(event.status_type.unwrap().as_u32(), raw);
    }
}

#[test]
fn raw_packet_fixtures_cover_accounting_statuses_and_attribute_preservation() {
    for (raw_packet, identifier, expected_status) in [
        (
            RAW_ACCOUNTING_START_FIXTURE.as_slice(),
            51,
            AcctStatusType::Start,
        ),
        (
            RAW_ACCOUNTING_INTERIM_FIXTURE.as_slice(),
            52,
            AcctStatusType::InterimUpdate,
        ),
        (
            RAW_ACCOUNTING_STOP_FIXTURE.as_slice(),
            53,
            AcctStatusType::Stop,
        ),
    ] {
        let request = handle_accounting_request(raw_packet).unwrap();
        let event = AccountingEvent::from_request(&request);

        assert_eq!(request.packet().code(), RadiusCode::AccountingRequest);
        assert_eq!(request.packet().identifier(), identifier);
        assert_eq!(event.status_type, Some(expected_status));
    }

    let request = handle_accounting_request(&RAW_UNKNOWN_VENDOR_IPV6_FIXTURE).unwrap();
    let event = AccountingEvent::from_request(&request);

    assert_unknown_standard_attributes(&event, &[(241, b"std")]);
    assert_eq!(
        event.unknown_vendor_attributes,
        vec![UnknownVendorAttribute {
            vendor_id: Some(12_345),
            vendor_type: None,
            value: b"vendor".to_vec(),
        }]
    );
    assert_eq!(event.nas_ipv6_address, Some("2001:db8::1".parse().unwrap()));
    assert_eq!(
        event.framed_ipv6_prefixes,
        vec![Ipv6Prefix {
            address: "2001:db8:10::".parse().unwrap(),
            prefix_len: 56,
        }]
    );
}

#[test]
fn extracts_session_nas_identity_and_subscriber_address_fields() {
    let request = request_with_attributes(&[
        radius_text_attribute(ACCT_SESSION_ID, "session-123"),
        radius_attribute(NAS_IP_ADDRESS, &[192, 0, 2, 10]),
        radius_attribute(
            NAS_IPV6_ADDRESS,
            &ipv6_bytes([0x2001, 0xdb8, 0, 0, 0, 0, 0, 1]),
        ),
        radius_text_attribute(NAS_IDENTIFIER, "nas-1"),
        radius_u32_attribute(NAS_PORT, 17),
        radius_text_attribute(NAS_PORT_ID, "ether5"),
        radius_text_attribute(USER_NAME, "subscriber@example.net"),
        radius_text_attribute(CALLING_STATION_ID, "00:11:22:33:44:55"),
        radius_text_attribute(CALLED_STATION_ID, "service"),
        radius_attribute(CLASS, b"class-data"),
        radius_attribute(CLASS, b"class-data-2"),
        radius_u32_attribute(EVENT_TIMESTAMP, 1_700_000_000),
        radius_attribute(FRAMED_IP_ADDRESS, &[198, 51, 100, 25]),
        radius_attribute(FRAMED_IP_NETMASK, &[255, 255, 255, 0]),
        radius_text_attribute(FRAMED_ROUTE, "198.51.100.0/24 192.0.2.1 1"),
        radius_text_attribute(FRAMED_ROUTE, "203.0.113.0/24 192.0.2.1 1"),
        ipv6_prefix_attribute(FRAMED_IPV6_PREFIX, 56, 0x2001, 0xdb8, 1),
        ipv6_prefix_attribute(DELEGATED_IPV6_PREFIX, 48, 0x2001, 0xdb8, 2),
        radius_attribute(
            FRAMED_IPV6_ADDRESS,
            &ipv6_bytes([0x2001, 0xdb8, 3, 0, 0, 0, 0, 5]),
        ),
    ]);
    let event = AccountingEvent::from_request(&request);

    assert_eq!(event.acct_session_id.as_deref(), Some("session-123"));
    assert_eq!(event.nas_ip_address, Some(Ipv4Addr::new(192, 0, 2, 10)));
    assert_eq!(event.nas_ipv6_address, Some("2001:db8::1".parse().unwrap()));
    assert_eq!(event.nas_identifier.as_deref(), Some("nas-1"));
    assert_eq!(event.nas_port, Some(17));
    assert_eq!(event.nas_port_id.as_deref(), Some("ether5"));
    assert_eq!(event.user_name.as_deref(), Some("subscriber@example.net"));
    assert_eq!(
        event.calling_station_id.as_deref(),
        Some("00:11:22:33:44:55")
    );
    assert_eq!(event.called_station_id.as_deref(), Some("service"));
    assert_eq!(
        event.class,
        vec![b"class-data".to_vec(), b"class-data-2".to_vec()]
    );
    assert_eq!(event.event_timestamp, Some(1_700_000_000));
    assert_eq!(
        event.framed_ip_address,
        Some(Ipv4Addr::new(198, 51, 100, 25))
    );
    assert_eq!(
        event.framed_ip_netmask,
        Some(Ipv4Addr::new(255, 255, 255, 0))
    );
    assert_eq!(
        event.framed_routes,
        vec![
            "198.51.100.0/24 192.0.2.1 1".to_string(),
            "203.0.113.0/24 192.0.2.1 1".to_string()
        ]
    );
    assert_eq!(
        event.framed_ipv6_prefixes,
        vec![Ipv6Prefix {
            address: "2001:db8:1::".parse().unwrap(),
            prefix_len: 56,
        }]
    );
    assert_eq!(
        event.delegated_ipv6_prefixes,
        vec![Ipv6Prefix {
            address: "2001:db8:2::".parse().unwrap(),
            prefix_len: 48,
        }]
    );
    assert_eq!(
        event.framed_ipv6_address,
        Some("2001:db8:3::5".parse().unwrap())
    );
    assert_eq!(event.unknown_standard_attributes, Vec::new());
    assert_eq!(event.unknown_vendor_attributes, Vec::new());
}

#[test]
fn extracts_compact_ipv6_prefix_fields() {
    let request = request_with_attributes(&[
        compact_ipv6_prefix_attribute(FRAMED_IPV6_PREFIX, 0, &[]),
        compact_ipv6_prefix_attribute(FRAMED_IPV6_PREFIX, 56, &[0x20, 0x01, 0x0d, 0xb8, 0, 1, 0]),
        compact_ipv6_prefix_attribute(
            DELEGATED_IPV6_PREFIX,
            64,
            &[0x20, 0x01, 0x0d, 0xb8, 0, 2, 0, 0],
        ),
        compact_ipv6_prefix_attribute(
            DELEGATED_IPV6_PREFIX,
            128,
            &ipv6_bytes([0x2001, 0xdb8, 3, 0, 0, 0, 0, 1]),
        ),
        compact_ipv6_prefix_attribute(
            FRAMED_IPV6_PREFIX,
            60,
            &[0x20, 0x01, 0x0d, 0xb8, 0, 4, 0xab, 0xc0, 0],
        ),
    ]);
    let event = AccountingEvent::from_request(&request);

    assert_eq!(
        event.framed_ipv6_prefixes,
        vec![
            Ipv6Prefix {
                address: "::".parse().unwrap(),
                prefix_len: 0,
            },
            Ipv6Prefix {
                address: "2001:db8:1::".parse().unwrap(),
                prefix_len: 56,
            },
            Ipv6Prefix {
                address: "2001:db8:4:abc0::".parse().unwrap(),
                prefix_len: 60,
            },
        ]
    );
    assert_eq!(
        event.delegated_ipv6_prefixes,
        vec![
            Ipv6Prefix {
                address: "2001:db8:2::".parse().unwrap(),
                prefix_len: 64,
            },
            Ipv6Prefix {
                address: "2001:db8:3::1".parse().unwrap(),
                prefix_len: 128,
            },
        ]
    );
    assert_eq!(event.unknown_standard_attributes, Vec::new());
}

#[test]
fn preserves_known_standard_attributes_that_fail_typed_decoding() {
    let request = request_with_attributes(&[
        radius_attribute(NAS_IP_ADDRESS, &[192, 0, 2]),
        radius_attribute(NAS_IPV6_ADDRESS, &[0; 15]),
        radius_attribute(ACCT_STATUS_TYPE, &[1, 2, 3]),
        radius_attribute(FRAMED_IPV6_ADDRESS, &[0; 15]),
        compact_ipv6_prefix_attribute(FRAMED_IPV6_PREFIX, 64, &[0x20, 0x01]),
        compact_ipv6_prefix_attribute(
            DELEGATED_IPV6_PREFIX,
            60,
            &[0x20, 0x01, 0x0d, 0xb8, 0, 4, 0xab, 0xcd],
        ),
        compact_ipv6_prefix_attribute(
            FRAMED_IPV6_PREFIX,
            56,
            &[0x20, 0x01, 0x0d, 0xb8, 0, 1, 0, 1],
        ),
        compact_ipv6_prefix_attribute_with_reserved(
            FRAMED_IPV6_PREFIX,
            1,
            64,
            &[0x20, 0x01, 0x0d, 0xb8, 0, 5, 0, 0],
        ),
    ]);
    let event = AccountingEvent::from_request(&request);

    assert_eq!(event.nas_ip_address, None);
    assert_eq!(event.nas_ipv6_address, None);
    assert_eq!(event.status_type, None);
    assert_eq!(event.framed_ipv6_address, None);
    assert_eq!(event.framed_ipv6_prefixes, Vec::new());
    assert_eq!(event.delegated_ipv6_prefixes, Vec::new());
    assert_unknown_standard_attributes(
        &event,
        &[
            (NAS_IP_ADDRESS, &[192, 0, 2][..]),
            (NAS_IPV6_ADDRESS, &[0; 15][..]),
            (ACCT_STATUS_TYPE, &[1, 2, 3][..]),
            (FRAMED_IPV6_ADDRESS, &[0; 15][..]),
            (FRAMED_IPV6_PREFIX, &[0, 64, 0x20, 0x01][..]),
            (
                DELEGATED_IPV6_PREFIX,
                &[0, 60, 0x20, 0x01, 0x0d, 0xb8, 0, 4, 0xab, 0xcd][..],
            ),
            (
                FRAMED_IPV6_PREFIX,
                &[0, 56, 0x20, 0x01, 0x0d, 0xb8, 0, 1, 0, 1][..],
            ),
            (
                FRAMED_IPV6_PREFIX,
                &[1, 64, 0x20, 0x01, 0x0d, 0xb8, 0, 5, 0, 0][..],
            ),
        ],
    );
}

#[test]
fn extracts_from_verified_accounting_request() {
    let attributes = radius_attributes(&[radius_text_attribute(ACCT_SESSION_ID, "verified")]);
    let packet = signed_accounting_request_packet(21, &attributes, SHARED_SECRET);
    let verified =
        verify_accounting_request(&packet, SHARED_SECRET, MessageAuthenticatorPolicy::Optional)
            .unwrap();
    let event = AccountingEvent::from_verified(&verified);

    assert_eq!(event.acct_session_id.as_deref(), Some("verified"));
}

#[test]
fn preserves_unknown_standard_and_vendor_attributes() {
    let request = request_with_attributes(&[
        radius_attribute(241, b"standard"),
        radius_attribute(MESSAGE_AUTHENTICATOR_TYPE, &[0_u8; 16]),
        radius_raw_vendor_attribute(12_345, b"opaque-vendor"),
        radius_raw_vendor_attribute(54_321, b""),
        radius_attribute(VENDOR_SPECIFIC, &[0, 0, 0]),
    ]);
    let event = AccountingEvent::from_request(&request);

    assert_unknown_standard_attributes(
        &event,
        &[
            (241, b"standard"),
            (MESSAGE_AUTHENTICATOR_TYPE, &[0_u8; 16][..]),
        ],
    );
    assert_eq!(
        event.unknown_vendor_attributes,
        vec![
            UnknownVendorAttribute {
                vendor_id: Some(12_345),
                vendor_type: None,
                value: b"opaque-vendor".to_vec(),
            },
            UnknownVendorAttribute {
                vendor_id: Some(54_321),
                vendor_type: None,
                value: Vec::new(),
            },
            UnknownVendorAttribute {
                vendor_id: None,
                vendor_type: None,
                value: vec![0, 0, 0],
            }
        ]
    );
}

#[test]
fn decodes_mikrotik_rate_limit_with_normal_direction_mapping() {
    let request = request_with_attributes(&[radius_vendor_attribute(
        MIKROTIK_VENDOR_ID,
        MIKROTIK_RATE_LIMIT,
        b"10M/25M 12M/30M 8M/20M 10/10 8 5M/15M",
    )]);
    let event = AccountingEvent::from_request(&request);

    assert_eq!(event.unknown_vendor_attributes, Vec::new());
    assert_eq!(
        event.mikrotik_rate_limits,
        vec![MikrotikRateLimit {
            original: "10M/25M 12M/30M 8M/20M 10/10 8 5M/15M".to_string(),
            nas_rx_bps: 10_000_000,
            nas_tx_bps: 25_000_000,
            upload_bps: 10_000_000,
            download_bps: 25_000_000,
        }]
    );
}

#[test]
fn decodes_mikrotik_rate_limit_with_swapped_direction_mapping() {
    let request = request_with_attributes(&[radius_vendor_attribute(
        MIKROTIK_VENDOR_ID,
        MIKROTIK_RATE_LIMIT,
        b"512k/2M",
    )]);
    let options = AccountingEventOptions {
        mikrotik_rate_limit_direction: MikrotikRateLimitDirection::Swapped,
    };
    let event = AccountingEvent::from_request_with_options(&request, options);

    assert_eq!(event.mikrotik_rate_limits.len(), 1);
    assert_eq!(event.mikrotik_rate_limits[0].nas_rx_bps, 512_000);
    assert_eq!(event.mikrotik_rate_limits[0].nas_tx_bps, 2_000_000);
    assert_eq!(event.mikrotik_rate_limits[0].upload_bps, 2_000_000);
    assert_eq!(event.mikrotik_rate_limits[0].download_bps, 512_000);
}

#[test]
fn preserves_unknown_and_invalid_mikrotik_subattributes() {
    let request = request_with_attributes(&[radius_vendor_attributes(
        MIKROTIK_VENDOR_ID,
        &[
            radius_vendor_subattribute(MIKROTIK_RATE_LIMIT, b"10M/25M"),
            radius_vendor_subattribute(99, b"opaque"),
            radius_vendor_subattribute(MIKROTIK_RATE_LIMIT, b"not-a-rate"),
        ],
    )]);
    let event = AccountingEvent::from_request(&request);

    assert_eq!(event.mikrotik_rate_limits.len(), 1);
    assert_eq!(event.mikrotik_rate_limits[0].upload_bps, 10_000_000);
    assert_eq!(event.mikrotik_rate_limits[0].download_bps, 25_000_000);
    assert_eq!(
        event.unknown_vendor_attributes,
        vec![
            UnknownVendorAttribute {
                vendor_id: Some(MIKROTIK_VENDOR_ID),
                vendor_type: Some(99),
                value: b"opaque".to_vec(),
            },
            UnknownVendorAttribute {
                vendor_id: Some(MIKROTIK_VENDOR_ID),
                vendor_type: Some(MIKROTIK_RATE_LIMIT),
                value: b"not-a-rate".to_vec(),
            },
        ]
    );
}

#[test]
fn preserves_malformed_mikrotik_subattributes() {
    let request = request_with_attributes(&[radius_raw_vendor_attribute(
        MIKROTIK_VENDOR_ID,
        &[MIKROTIK_RATE_LIMIT, 1],
    )]);
    let event = AccountingEvent::from_request(&request);

    assert_eq!(event.mikrotik_rate_limits, Vec::new());
    assert_eq!(
        event.unknown_vendor_attributes,
        vec![UnknownVendorAttribute {
            vendor_id: Some(MIKROTIK_VENDOR_ID),
            vendor_type: Some(MIKROTIK_RATE_LIMIT),
            value: vec![MIKROTIK_RATE_LIMIT, 1],
        }]
    );
}

fn assert_unknown_standard_attributes(event: &AccountingEvent, expected: &[(u8, &[u8])]) {
    let actual: Vec<_> = event
        .unknown_standard_attributes
        .iter()
        .map(|attribute| (attribute.kind(), attribute.value()))
        .collect();

    assert_eq!(actual.as_slice(), expected);
}

fn request_with_attributes(attributes: &[Vec<u8>]) -> AccountingRequest {
    let encoded = radius_attributes(attributes);
    handle_accounting_request(&accounting_request_packet(7, &encoded)).unwrap()
}

fn ipv6_prefix_attribute(kind: u8, prefix_len: u8, a: u16, b: u16, c: u16) -> Vec<u8> {
    let mut value = Vec::with_capacity(18);
    value.push(0);
    value.push(prefix_len);
    value.extend_from_slice(&ipv6_bytes([a, b, c, 0, 0, 0, 0, 0]));
    radius_attribute(kind, &value)
}

fn compact_ipv6_prefix_attribute(kind: u8, prefix_len: u8, prefix_bytes: &[u8]) -> Vec<u8> {
    compact_ipv6_prefix_attribute_with_reserved(kind, 0, prefix_len, prefix_bytes)
}

fn compact_ipv6_prefix_attribute_with_reserved(
    kind: u8,
    reserved: u8,
    prefix_len: u8,
    prefix_bytes: &[u8],
) -> Vec<u8> {
    let mut value = Vec::with_capacity(2 + prefix_bytes.len());
    value.push(reserved);
    value.push(prefix_len);
    value.extend_from_slice(prefix_bytes);
    radius_attribute(kind, &value)
}

fn ipv6_bytes(segments: [u16; 8]) -> [u8; 16] {
    let mut bytes = [0_u8; 16];
    for (index, segment) in segments.into_iter().enumerate() {
        bytes[index * 2..index * 2 + 2].copy_from_slice(&segment.to_be_bytes());
    }
    bytes
}
