use crate::bridge_analytics;
use crate::bridge_analytics::flow_history::FlowHistorySnapshot;
use lqos_utils::units::DownUpOrder;

/// A Flow Globe wire row:
/// `(lat, lng, country, bytes_down, rtt_internet_nanos, bytes_up)`.
///
/// The RTT is the **INTERNET** leg (`rtt_nanos_up`) — the WAN-anchored
/// direction per the `.47` triad (down = Access at the subscriber tap,
/// up = Internet at the WAN; `circuit_activity.rs`). The hexbin colours by the
/// per-cell MEDIAN Internet RTT and sizes by total (down + up) throughput
/// (`docs/flow_globe_hexbin_card.md` §3 WG1; DRIVER override 2026-10-05).
pub type FlowMapRow = (f64, f64, String, u64, f32, u64);

pub fn flow_map_data() -> Vec<FlowMapRow> {
    if let Some(snapshot) = bridge_analytics::flow_history_snapshot() {
        return flow_map_from_history(&snapshot);
    }
    Vec::new() // the bridge flow history is the source
}

pub fn endpoints_by_country_data() -> Vec<(String, DownUpOrder<u64>, [f32; 2], String)> {
    if let Some(snapshot) = bridge_analytics::flow_history_snapshot() {
        return country_summary_from_history(&snapshot);
    }
    Vec::new() // the bridge flow history is the source
}

/// The legacy 5-field endpoint rows `(lat, lng, country, bytes_down,
/// rtt_internet)` for the `EndpointLatLon` ticker (the world-map dashlet). The
/// RTT is the INTERNET leg, the same value the Flow Globe colours by.
pub fn endpoint_latlon_data() -> Vec<(f64, f64, String, u64, f32)> {
    flow_map_data()
        .into_iter()
        .map(|(lat, lng, country, bytes_down, rtt_internet, _bytes_up)| {
            (lat, lng, country, bytes_down, rtt_internet)
        })
        .collect()
}

/// Cluster-2: lat/lon endpoints from the retained finished flows, **aggregated
/// by exact location**. The raw finished-flow set is dominated by duplicate
/// coordinates (the live jackrabbit payload: 9,754 rows but only 142 distinct
/// `(lat,lng)`), which makes the downstream top-5000-by-throughput cap spend
/// ~98% of its slots on repeats and starve the regional tail. One row per
/// location keeps the SAME 6-tuple wire shape: bytes summed, RTT = the MEDIAN
/// of the nonzero Internet-leg (`rtt_nanos_up`) samples. The RTT is the INTERNET
/// leg; the bytes carry both directions.
fn flow_map_from_history(snapshot: &FlowHistorySnapshot) -> Vec<FlowMapRow> {
    use std::collections::BTreeMap;

    // Keyed by the exact bit pattern of `(lat, lon)`; the value carries the
    // first-seen display country, the byte sums, and the nonzero RTT samples.
    let mut groups: BTreeMap<(u64, u64), (f64, f64, String, u64, Vec<f32>, u64)> =
        BTreeMap::new();
    for flow in snapshot.recent.iter() {
        if flow.lat == 0.0 || flow.lon == 0.0 {
            continue; // zero-geo filter (unchanged)
        }
        let entry = groups
            .entry((flow.lat.to_bits(), flow.lon.to_bits()))
            .or_insert_with(|| {
                (
                    flow.lat,
                    flow.lon,
                    flow.country_name.clone(),
                    0,
                    Vec::new(),
                    0,
                )
            });
        entry.3 = entry.3.saturating_add(flow.bytes.down);
        entry.5 = entry.5.saturating_add(flow.bytes.up);
        if flow.rtt_nanos_up > 0 {
            entry.4.push(flow.rtt_nanos_up as f32);
        }
    }

    let mut points: Vec<FlowMapRow> = groups
        .into_values()
        .map(|(lat, lon, country, down, rtt_nanos_up, up)| {
            (lat, lon, country, down, median_f32(&rtt_nanos_up), up)
        })
        .collect();
    points.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));
    points
}

/// Cluster-2: per-country summary from the retained finished flows (the
/// kernel `country_summary` shape: country name, total bytes, [down,up]
/// median RTT, ISO flag), sorted by down bytes descending.
fn country_summary_from_history(
    snapshot: &FlowHistorySnapshot,
) -> Vec<(String, DownUpOrder<u64>, [f32; 2], String)> {
    let mut groups: std::collections::BTreeMap<String, (String, DownUpOrder<u64>, Vec<f32>)> =
        std::collections::BTreeMap::new();
    for flow in snapshot.recent.iter() {
        let entry = groups
            .entry(flow.country_name.clone())
            .or_insert_with(|| (flow.country_iso.clone(), DownUpOrder::zeroed(), Vec::new()));
        entry.1.down = entry.1.down.saturating_add(flow.bytes.down);
        entry.1.up = entry.1.up.saturating_add(flow.bytes.up);
        if flow.rtt_nanos > 0 {
            entry.2.push(flow.rtt_nanos as f32);
        }
    }
    let mut summary: Vec<(String, DownUpOrder<u64>, [f32; 2], String)> = groups
        .into_iter()
        .map(|(name, (iso, bytes, rtt_nanos))| {
            let median = median_f32(&rtt_nanos);
            (name, bytes, [median, 0.0], iso)
        })
        .collect();
    summary.sort_by_key(|row| std::cmp::Reverse(row.1.down));
    summary
}

fn median_f32(slice: &[f32]) -> f32 {
    if slice.is_empty() {
        return 0.0;
    }
    let mut slice = slice.to_vec();
    slice.sort_by(|a, b| a.total_cmp(b));
    let mid = slice.len() / 2;
    if slice.len().is_multiple_of(2) {
        (slice[mid] + slice[mid - 1]) / 2.0
    } else {
        slice[mid]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge_analytics::flow_history::RecentFlow;
    use analytics_ipc::{FlowId, FlowState};

    fn flow(lat: f64, lon: f64, country: &str, iso: &str, down: u64, rtt: u64) -> RecentFlow {
        RecentFlow {
            flow_id: FlowId {
                circuit_id: 1,
                flow_hash: down,
                subscriber_ip: "192.0.2.1".parse().expect("test IP"),
                remote_ip: "198.51.100.1".parse().expect("test IP"),
                subscriber_port: 1,
                remote_port: 443,
                protocol: 6,
            },
            state: FlowState::Fin,
            first_seen_millis: 0,
            last_seen_millis: 5_000,
            bytes: DownUpOrder::new(down, 10),
            asn: Some(64500),
            country_iso: iso.to_string(),
            country_name: country.to_string(),
            protocol: "HTTPS".to_string(),
            lat,
            lon,
            rtt_nanos: rtt,
            rtt_nanos_down: rtt,
            rtt_nanos_up: rtt,
            is_v4: true,
            packets: DownUpOrder::new(down, 10),
            tcp_retransmits: 0,
        }
    }

    #[test]
    fn history_flow_map_filters_zero_geo_and_sorts() {
        let snapshot = FlowHistorySnapshot {
            recent: vec![
                flow(2.0, 3.0, "France", "FR", 100, 1_000),
                flow(0.0, 0.0, "Nowhere", "XX", 50, 1_000),
                flow(1.0, 4.0, "France", "FR", 200, 3_000),
            ]
            .into(),
            ..Default::default()
        };
        let points = flow_map_from_history(&snapshot);
        assert_eq!(points.len(), 2, "the zero geo point is dropped");
        assert_eq!(points[0].0, 1.0, "sorted by latitude");
        let summary = country_summary_from_history(&snapshot);
        assert_eq!(summary.len(), 2, "the country summary keeps every country");
        let france = summary
            .iter()
            .find(|row| row.0 == "France")
            .expect("France present");
        assert_eq!(france.1.down, 300, "bytes fold per country");
        assert_eq!(france.2[0], 2_000.0, "median RTT");
    }

    #[test]
    fn the_flow_map_row_carries_the_internet_leg_and_up_bytes() {
        let mut f = flow(1.0, 2.0, "France", "FR", 100, 9_999);
        f.rtt_nanos = 9_999; // combined / full
        f.rtt_nanos_down = 1_000; // Access (the subscriber tap)
        f.rtt_nanos_up = 42_000; // Internet (the WAN)
        f.bytes = DownUpOrder::new(100, 7);
        let snapshot = FlowHistorySnapshot {
            recent: vec![f].into(),
            ..Default::default()
        };
        let points = flow_map_from_history(&snapshot);
        assert_eq!(points.len(), 1);
        let row = &points[0];
        assert_eq!(row.0, 1.0);
        assert_eq!(row.3, 100, "down bytes");
        assert_eq!(
            row.4, 42_000.0,
            "the wire RTT must be the INTERNET leg (rtt_nanos_up), NOT the combined rtt_nanos"
        );
        assert_eq!(row.5, 7, "up bytes");
    }

    #[test]
    fn history_flow_map_aggregates_duplicate_locations() {
        let snapshot = FlowHistorySnapshot {
            recent: vec![
                flow(10.0, 20.0, "US", "US", 100, 1_000),
                flow(10.0, 20.0, "US", "US", 200, 3_000),
                flow(10.0, 20.0, "US", "US", 50, 0), // zero RTT excluded from the median
                flow(30.0, 40.0, "FR", "FR", 7, 9_000),
                flow(50.0, 60.0, "XX", "XX", 5, 0), // all-zero RTT ⇒ median 0
            ]
            .into(),
            ..Default::default()
        };
        let points = flow_map_from_history(&snapshot);

        assert_eq!(points.len(), 3, "one row per distinct location");
        // Sorted by (lat, lng).
        assert_eq!((points[0].0, points[0].1), (10.0, 20.0));
        assert_eq!((points[1].0, points[1].1), (30.0, 40.0));
        assert_eq!((points[2].0, points[2].1), (50.0, 60.0));

        let us = &points[0];
        assert_eq!(us.2, "US", "first-seen country retained");
        assert_eq!(us.3, 350, "down bytes summed across the location");
        assert_eq!(us.5, 30, "up bytes summed across the location");
        assert_eq!(
            us.4, 2_000.0,
            "median of the NONZERO rtt_nanos_up samples (1000, 3000)"
        );

        assert_eq!(points[1].3, 7, "a distinct location keeps its own bytes");
        assert_eq!(
            points[2].4, 0.0,
            "an all-zero-RTT location medians to 0 (no samples)"
        );
    }

    #[test]
    fn history_flow_map_is_empty_for_an_empty_snapshot() {
        let snapshot = FlowHistorySnapshot::default();
        assert!(
            flow_map_from_history(&snapshot).is_empty(),
            "the cold path (no finished flows) yields no rows"
        );
    }
}
