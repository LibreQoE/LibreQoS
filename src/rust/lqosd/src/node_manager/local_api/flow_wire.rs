//! The shared WIRE mappers over the bridge flow-history snapshot.
//!
//! The DATA accessors live on [`FlowHistorySnapshot`] (`bridge_analytics`);
//! these turn that data into the exact `BusResponse` / list shapes the bus and
//! the websocket endpoints publish. Keeping them here (rather than on the
//! snapshot) avoids a `bridge_analytics -> node_manager` layering inversion.
//!
//! The shapes are the frozen wire contract inherited from the retired
//! `throughput_tracker::flow_data` recent-flows cache: the ether summary's
//! v4/v6 bytes + packets + per-direction median RTTs, the flow-duration
//! histogram, and the flow-explorer lists.

use crate::bridge_analytics::flow_history::{FlowHistorySnapshot, RecentFlow};
use crate::throughput_tracker::flow_data::{
    AsnCountryListEntry, AsnListEntry, AsnProtocolListEntry, get_asn_name_by_id,
};
use itertools::Itertools;
use lqos_bus::BusResponse;
use lqos_utils::units::DownUpOrder;

/// The list views' minimum flow duration (the legacy window: 3 s).
const LIST_MIN_DURATION_MILLIS: u64 = 3_000;

/// The legacy `ether_protocol_summary` wire shape over `snapshot.recent`: the
/// v4/v6 split (by `is_v4`) with per-direction bytes, packets and median RTT
/// (nanos). The per-direction RTT medians use the legacy `> 0` filter.
pub(crate) fn ether_protocol_summary(snapshot: &FlowHistorySnapshot) -> BusResponse {
    let mut v4_bytes = DownUpOrder::zeroed();
    let mut v4_packets = DownUpOrder::zeroed();
    let mut v6_bytes = DownUpOrder::zeroed();
    let mut v6_packets = DownUpOrder::zeroed();
    let mut v4_rtt = [Vec::new(), Vec::new()];
    let mut v6_rtt = [Vec::new(), Vec::new()];

    for flow in snapshot.recent.iter() {
        let (bytes, packets, rtt) = if flow.is_v4 {
            (&mut v4_bytes, &mut v4_packets, &mut v4_rtt)
        } else {
            (&mut v6_bytes, &mut v6_packets, &mut v6_rtt)
        };
        bytes.checked_add(flow.bytes);
        packets.checked_add(flow.packets);
        if flow.rtt_nanos_down > 0 {
            rtt[0].push(flow.rtt_nanos_down);
        }
        if flow.rtt_nanos_up > 0 {
            rtt[1].push(flow.rtt_nanos_up);
        }
    }

    BusResponse::EtherProtocols {
        v4_bytes,
        v6_bytes,
        v4_packets,
        v6_packets,
        v4_rtt: DownUpOrder::new(median_u64(&v4_rtt[0]), median_u64(&v4_rtt[1])),
        v6_rtt: DownUpOrder::new(median_u64(&v6_rtt[0]), median_u64(&v6_rtt[1])),
    }
}

/// The legacy `flow_duration_summary` wire shape: the same arithmetic (whole
/// seconds, sorted, deduplicated with counts) over the retained flows.
pub(crate) fn flow_duration_summary(snapshot: &FlowHistorySnapshot) -> Vec<(usize, u64)> {
    snapshot
        .recent
        .iter()
        .map(|flow| flow.last_seen_millis.saturating_sub(flow.first_seen_millis) / 1_000)
        .sorted()
        .dedup_with_count()
        .collect()
}

/// Builds a list of all ASNs with recent data, and how many flows they have.
pub(crate) fn asn_list(snapshot: &FlowHistorySnapshot) -> Vec<AsnListEntry> {
    let mut counts: std::collections::BTreeMap<u32, usize> = std::collections::BTreeMap::new();
    for flow in snapshot.recent.iter() {
        if !is_long_enough(flow, LIST_MIN_DURATION_MILLIS) {
            continue;
        }
        *counts.entry(flow.asn.unwrap_or(0)).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|(asn, count)| AsnListEntry {
            count,
            asn,
            name: get_asn_name_by_id(asn),
        })
        .collect()
}

/// Builds a list of countries with recent data, and how many flows they have.
pub(crate) fn country_list(snapshot: &FlowHistorySnapshot) -> Vec<AsnCountryListEntry> {
    let mut counts: std::collections::BTreeMap<(String, String), usize> =
        std::collections::BTreeMap::new();
    for flow in snapshot.recent.iter() {
        if !is_long_enough(flow, LIST_MIN_DURATION_MILLIS) {
            continue;
        }
        *counts
            .entry((flow.country_name.clone(), flow.country_iso.clone()))
            .or_default() += 1;
    }
    counts
        .into_iter()
        .map(|((name, iso_code), count)| AsnCountryListEntry {
            count,
            name,
            iso_code,
        })
        .collect()
}

/// Builds a list of protocols with recent data, and how many flows they have.
pub(crate) fn protocol_list(snapshot: &FlowHistorySnapshot) -> Vec<AsnProtocolListEntry> {
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for flow in snapshot.recent.iter() {
        if !is_long_enough(flow, LIST_MIN_DURATION_MILLIS) {
            continue;
        }
        *counts.entry(flow.protocol.clone()).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|(protocol, count)| AsnProtocolListEntry { count, protocol })
        .collect()
}

/// The list/timeline duration window: whole milliseconds in `recent`.
pub(crate) fn is_long_enough(flow: &RecentFlow, minimum_millis: u64) -> bool {
    flow.last_seen_millis.saturating_sub(flow.first_seen_millis) > minimum_millis
}

fn median_u64(slice: &[u64]) -> u64 {
    if slice.is_empty() {
        return 0;
    }
    let mut slice = slice.to_vec();
    slice.sort_unstable();
    let mid = slice.len() / 2;
    if slice.len().is_multiple_of(2) {
        (slice[mid] + slice[mid - 1]) / 2
    } else {
        slice[mid]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use analytics_ipc::{FlowId, FlowState};

    fn flow(
        flow_hash: u64,
        asn: Option<u32>,
        country_iso: &str,
        protocol: &str,
        first_millis: u64,
        last_millis: u64,
    ) -> RecentFlow {
        RecentFlow {
            flow_id: FlowId {
                circuit_id: 1,
                flow_hash,
                subscriber_ip: "192.0.2.1".parse().expect("test IP"),
                remote_ip: "198.51.100.1".parse().expect("test IP"),
                subscriber_port: 12_345,
                remote_port: 443,
                protocol: 6,
            },
            state: FlowState::Fin,
            first_seen_millis: first_millis,
            last_seen_millis: last_millis,
            bytes: DownUpOrder::new(1_000, 250),
            asn,
            country_iso: country_iso.to_string(),
            country_name: "Nowhere".to_string(),
            protocol: protocol.to_string(),
            lat: 1.0,
            lon: 2.0,
            rtt_nanos: 5_000,
            rtt_nanos_down: 5_000,
            rtt_nanos_up: 5_000,
            is_v4: true,
            packets: DownUpOrder::new(1_000, 250),
            tcp_retransmits: 3,
        }
    }

    #[test]
    fn history_lists_group_and_filter_like_the_kernel() {
        let snapshot = FlowHistorySnapshot {
            recent: vec![
                flow(1, Some(64500), "US", "HTTPS", 0, 5_000),
                flow(2, Some(64500), "US", "HTTPS", 0, 1_000),
                flow(3, Some(64501), "DE", "DNS", 0, 9_000),
            ]
            .into(),
            ..Default::default()
        };
        let asns = asn_list(&snapshot);
        assert_eq!(asns.len(), 2, "short flows are filtered from the list");
        assert_eq!(asns[0].asn, 64500);
        assert_eq!(asns[0].count, 1);

        let countries = country_list(&snapshot);
        assert_eq!(countries.iter().map(|c| c.count).sum::<usize>(), 2);
        let protocols = protocol_list(&snapshot);
        assert!(
            protocols
                .iter()
                .any(|row| row.protocol == "HTTPS" && row.count == 1),
            "the long HTTPS flow is listed once: {protocols:?}"
        );
    }

    #[test]
    fn ether_protocol_summary_freezes_the_wire_shape() {
        let mut v4a = flow(1, Some(64500), "US", "HTTPS", 0, 5_000);
        v4a.bytes = DownUpOrder::new(100, 10);
        v4a.packets = DownUpOrder::new(5, 1);
        v4a.rtt_nanos_down = 3_000;
        v4a.rtt_nanos_up = 0;
        let mut v4b = flow(2, Some(64500), "US", "HTTPS", 0, 5_000);
        v4b.bytes = DownUpOrder::new(200, 20);
        v4b.packets = DownUpOrder::new(7, 2);
        v4b.rtt_nanos_down = 1_000;
        v4b.rtt_nanos_up = 5_000;
        let mut v6 = flow(3, Some(64501), "DE", "DNS", 0, 5_000);
        v6.bytes = DownUpOrder::new(50, 5);
        v6.packets = DownUpOrder::new(3, 0);
        v6.rtt_nanos_down = 0;
        v6.rtt_nanos_up = 2_000;
        v6.is_v4 = false;

        let snapshot = FlowHistorySnapshot {
            recent: vec![v4a, v4b, v6].into(),
            ..Default::default()
        };

        assert_eq!(
            ether_protocol_summary(&snapshot),
            BusResponse::EtherProtocols {
                v4_bytes: DownUpOrder::new(300, 30),
                v6_bytes: DownUpOrder::new(50, 5),
                v4_packets: DownUpOrder::new(12, 3),
                v6_packets: DownUpOrder::new(3, 0),
                v4_rtt: DownUpOrder::new(2_000, 5_000),
                v6_rtt: DownUpOrder::new(0, 2_000),
            }
        );
    }

    #[test]
    fn flow_duration_summary_reproduces_the_kernel_histogram() {
        let snapshot = FlowHistorySnapshot {
            recent: vec![
                flow(1, Some(1), "US", "HTTPS", 0, 5_000),
                flow(2, Some(1), "US", "HTTPS", 0, 1_000),
                flow(3, Some(1), "US", "HTTPS", 100, 300),
                flow(4, Some(1), "US", "HTTPS", 0, 5_000),
            ]
            .into(),
            ..Default::default()
        };
        assert_eq!(
            flow_duration_summary(&snapshot),
            vec![(1, 0), (1, 1), (2, 5)],
            "whole seconds, sorted, deduplicated with counts"
        );
    }
}
