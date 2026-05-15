//! Host-side packet policy fixtures for the eBPF dissector.
//!
//! These tests pin the conservative behavior that malformed or complex packets
//! pass without shaping/flow tracking when the BPF dissector cannot safely find
//! a direct transport header.

const ETH_P_IP: u16 = 0x0800;
const ETH_P_IPV6: u16 = 0x86dd;
const ETH_P_MPLS_UC: u16 = 0x8847;
const ETH_HEADER_LEN: usize = 14;
const IPV6_HEADER_LEN: usize = 40;
const IPV6_NEXT_HEADER_OFFSET: usize = 6;
const IPV6_NEXTHDR_HOP_BY_HOP: u8 = 0;
const IPV6_NEXTHDR_ROUTING: u8 = 43;
const IPV6_NEXTHDR_FRAGMENT: u8 = 44;
const IPV6_NEXTHDR_AUTHENTICATION: u8 = 51;
const IPV6_NEXTHDR_DESTINATION_OPTIONS: u8 = 60;
const IPPROTO_TCP: u8 = 6;
const IPPROTO_UDP: u8 = 17;
const MPLS_BOTTOM_OF_STACK: u32 = 0x0000_0100;

#[derive(Debug, PartialEq, Eq)]
enum PacketPolicy {
    LookupEligible,
    PassUnshaped,
}

fn ipv6_next_header_passes_unshaped(next_header: u8) -> bool {
    matches!(
        next_header,
        IPV6_NEXTHDR_HOP_BY_HOP
            | IPV6_NEXTHDR_ROUTING
            | IPV6_NEXTHDR_FRAGMENT
            | IPV6_NEXTHDR_AUTHENTICATION
            | IPV6_NEXTHDR_DESTINATION_OPTIONS
    )
}

fn classify_fixture_frame(frame: &[u8]) -> PacketPolicy {
    if frame.len() < ETH_HEADER_LEN {
        return PacketPolicy::PassUnshaped;
    }

    let eth_type = u16::from_be_bytes([frame[12], frame[13]]);
    let l3_offset = ETH_HEADER_LEN;

    match eth_type {
        ETH_P_IP => PacketPolicy::LookupEligible,
        ETH_P_IPV6 => classify_ipv6_fixture(frame, l3_offset),
        // Standards-compliant MPLS frames remain fail-open in the eBPF policy
        // until a bounded, verifier-friendly parser is added.
        ETH_P_MPLS_UC => PacketPolicy::PassUnshaped,
        _ => PacketPolicy::PassUnshaped,
    }
}

fn classify_ipv6_fixture(frame: &[u8], l3_offset: usize) -> PacketPolicy {
    let Some(header_end) = l3_offset.checked_add(IPV6_HEADER_LEN) else {
        return PacketPolicy::PassUnshaped;
    };
    if frame.len() < header_end {
        return PacketPolicy::PassUnshaped;
    }

    let next_header = frame[l3_offset + IPV6_NEXT_HEADER_OFFSET];
    if ipv6_next_header_passes_unshaped(next_header) {
        PacketPolicy::PassUnshaped
    } else {
        PacketPolicy::LookupEligible
    }
}

fn ethernet_frame(eth_type: u16, payload: &[u8]) -> Vec<u8> {
    let mut frame = vec![0; ETH_HEADER_LEN];
    frame[12..14].copy_from_slice(&eth_type.to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

fn ipv6_packet(next_header: u8) -> Vec<u8> {
    let mut packet = vec![0; IPV6_HEADER_LEN];
    packet[0] = 0x60;
    packet[IPV6_NEXT_HEADER_OFFSET] = next_header;
    packet
}

fn mpls_label(bottom_of_stack: bool) -> [u8; 4] {
    let entry = if bottom_of_stack {
        MPLS_BOTTOM_OF_STACK
    } else {
        0
    };
    entry.to_be_bytes()
}

#[test]
fn ipv6_first_extension_headers_pass_unshaped() {
    for next_header in [
        IPV6_NEXTHDR_HOP_BY_HOP,
        IPV6_NEXTHDR_ROUTING,
        IPV6_NEXTHDR_FRAGMENT,
        IPV6_NEXTHDR_AUTHENTICATION,
        IPV6_NEXTHDR_DESTINATION_OPTIONS,
    ] {
        let frame = ethernet_frame(ETH_P_IPV6, &ipv6_packet(next_header));

        assert_eq!(classify_fixture_frame(&frame), PacketPolicy::PassUnshaped);
    }
}

#[test]
fn direct_ipv6_transport_headers_remain_lookup_eligible() {
    for next_header in [IPPROTO_TCP, IPPROTO_UDP] {
        let frame = ethernet_frame(ETH_P_IPV6, &ipv6_packet(next_header));

        assert_eq!(classify_fixture_frame(&frame), PacketPolicy::LookupEligible);
    }
}

#[test]
fn truncated_ipv6_headers_pass_unshaped() {
    let frame = ethernet_frame(ETH_P_IPV6, &[0x60, 0, 0, 0]);

    assert_eq!(classify_fixture_frame(&frame), PacketPolicy::PassUnshaped);
}

#[test]
fn stacked_mpls_fixtures_pass_unshaped() {
    let mut payload = Vec::new();
    payload.extend_from_slice(&mpls_label(false));
    payload.extend_from_slice(&mpls_label(true));
    payload.extend_from_slice(&ipv6_packet(IPPROTO_TCP));
    let frame = ethernet_frame(ETH_P_MPLS_UC, &payload);

    assert_eq!(classify_fixture_frame(&frame), PacketPolicy::PassUnshaped);
}
