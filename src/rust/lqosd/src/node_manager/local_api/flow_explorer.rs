use super::flow_wire;
use crate::bridge_analytics;
use crate::bridge_analytics::flow_history::{self, FlowHistorySnapshot, RecentFlow};
use crate::throughput_tracker::flow_data::{
    AsnCountryListEntry, AsnListEntry, AsnProtocolListEntry, FlowAnalysis, FlowbeeLocalData,
    RttData, retry_times_to_unix_seconds,
};
use crate::throughput_tracker::resolve_circuit_metadata_for_ip;
use lqos_sys::flowbee_data::FlowbeeKey;
use lqos_utils::XdpIpAddress;
use lqos_utils::units::DownUpOrder;
use lqos_utils::unix_time::{TimeError, time_since_boot, unix_now};
use serde::Serialize;
use std::time::Duration;

pub fn asn_list_data() -> Vec<AsnListEntry> {
    if let Some(snapshot) = bridge_analytics::flow_history_snapshot() {
        return flow_wire::asn_list(&snapshot);
    }
    Vec::new() // the bridge snapshot is the source
}

pub fn country_list_data() -> Vec<AsnCountryListEntry> {
    if let Some(snapshot) = bridge_analytics::flow_history_snapshot() {
        return flow_wire::country_list(&snapshot);
    }
    Vec::new()
}

pub fn protocol_list_data() -> Vec<AsnProtocolListEntry> {
    if let Some(snapshot) = bridge_analytics::flow_history_snapshot() {
        return flow_wire::protocol_list(&snapshot);
    }
    Vec::new()
}

/// The timeline views use the kernel's 2 s minimum.
const TIMELINE_MIN_DURATION_MILLIS: u64 = 2_000;

#[derive(Debug, Serialize)]
pub struct FlowTimeline {
    pub start: u64,
    pub end: u64,
    pub duration_nanos: u64,
    pub throughput: Vec<DownUpOrder<u64>>,
    pub tcp_retransmits: DownUpOrder<u16>,
    pub rtt: [RttData; 2],
    pub retransmit_times_down: Vec<u64>,
    pub retransmit_times_up: Vec<u64>,
    pub total_bytes: DownUpOrder<u64>,
    pub protocol: String,
    pub circuit_id: String,
    pub circuit_name: String,
    pub remote_ip: String,
}

/// Cluster-2 §8 fold 4: one relative-time bucket of the aggregate ring,
/// filtered to the requested class. Rides the SAME timeline responses as an
/// additive field (no new endpoint, no JS change).
#[derive(Debug, Serialize)]
pub struct FlowTimelineBucket {
    pub start_millis: u64,
    pub sampled_intervals: u16,
    pub bytes: DownUpOrder<u64>,
    pub flows: u64,
}

pub fn flow_timeline_data(asn_id: u32) -> Result<Vec<FlowTimeline>, TimeError> {
    if let Some(snapshot) = bridge_analytics::flow_history_snapshot() {
        let boot_time = flow_timeline_boot_time()?;
        return Ok(timeline_from_history(&snapshot, boot_time, |flow| {
            flow.asn.unwrap_or(0) == asn_id
        }));
    }
    let boot_time = flow_timeline_boot_time()?;

    let all_flows_for_asn = Vec::new();

    Ok(all_flows_to_transport(boot_time, all_flows_for_asn))
}

pub fn country_timeline_data(iso_code: &str) -> Result<Vec<FlowTimeline>, TimeError> {
    if let Some(snapshot) = bridge_analytics::flow_history_snapshot() {
        let boot_time = flow_timeline_boot_time()?;
        return Ok(timeline_from_history(&snapshot, boot_time, |flow| {
            flow.country_iso == iso_code
        }));
    }
    let boot_time = flow_timeline_boot_time()?;

    let all_flows_for_asn = Vec::new();

    Ok(all_flows_to_transport(boot_time, all_flows_for_asn))
}

pub fn protocol_timeline_data(protocol_name: &str) -> Result<Vec<FlowTimeline>, TimeError> {
    let protocol_name = protocol_name.replace("_", "/");
    if let Some(snapshot) = bridge_analytics::flow_history_snapshot() {
        let boot_time = flow_timeline_boot_time()?;
        return Ok(timeline_from_history(&snapshot, boot_time, |flow| {
            flow.protocol == protocol_name
        }));
    }
    let boot_time = flow_timeline_boot_time()?;

    let all_flows_for_asn = Vec::new();

    Ok(all_flows_to_transport(boot_time, all_flows_for_asn))
}

/// Cluster-2: the ring window (full 24 h) for the requested class, empty
/// when kernel-sourced.
pub fn asn_timeline_buckets(asn_id: u32) -> Vec<FlowTimelineBucket> {
    history_buckets(|snapshot| {
        snapshot.asn_buckets(Some(asn_id), flow_history::RING_ENTRIES)
    })
}

pub fn country_timeline_buckets(iso_code: &str) -> Vec<FlowTimelineBucket> {
    history_buckets(|snapshot| snapshot.country_buckets(iso_code, flow_history::RING_ENTRIES))
}

pub fn protocol_timeline_buckets(protocol_name: &str) -> Vec<FlowTimelineBucket> {
    history_buckets(|snapshot| {
        snapshot.protocol_buckets(protocol_name, flow_history::RING_ENTRIES)
    })
}

fn history_buckets(
    map: impl FnOnce(&FlowHistorySnapshot) -> Vec<flow_history::HistoryBucket>,
) -> Vec<FlowTimelineBucket> {
    match bridge_analytics::flow_history_snapshot() {
        Some(snapshot) => map(&snapshot)
            .into_iter()
            .map(|bucket| FlowTimelineBucket {
                start_millis: bucket.start_millis,
                sampled_intervals: bucket.sampled_intervals,
                bytes: DownUpOrder::new(bucket.bytes.down, bucket.bytes.up),
                flows: bucket.flows,
            })
            .collect(),
        None => Vec::new(),
    }
}

fn timeline_from_history<F>(
    snapshot: &FlowHistorySnapshot,
    boot_time: u64,
    include: F,
) -> Vec<FlowTimeline>
where
    F: Fn(&RecentFlow) -> bool,
{
    snapshot
        .recent
        .iter()
        .filter(|flow| include(flow))
        .filter(|flow| flow_wire::is_long_enough(flow, TIMELINE_MIN_DURATION_MILLIS))
        .map(|flow| recent_to_timeline(flow, boot_time))
        .collect()
}

fn recent_to_timeline(flow: &RecentFlow, boot_time: u64) -> FlowTimeline {
    let subscriber = XdpIpAddress::from_ip(flow.flow_id.subscriber_ip);
    let (circuit_id, mut circuit_name) = resolve_circuit_metadata_for_ip(&subscriber);
    if circuit_name.is_empty() {
        circuit_name = flow.flow_id.subscriber_ip.to_string();
    }
    FlowTimeline {
        start: boot_time + flow.first_seen_millis / 1_000,
        end: boot_time + flow.last_seen_millis / 1_000,
        duration_nanos: flow
            .last_seen_millis
            .saturating_sub(flow.first_seen_millis)
            .saturating_mul(1_000_000),
        // Cluster-2 named honest-empty: the bridge wire carries no per-flow
        // throughput time-series (the ring is per-CLASS, not per-flow).
        throughput: vec![],
        tcp_retransmits: DownUpOrder::new(
            flow.tcp_retransmits.min(u64::from(u16::MAX)) as u16,
            0,
        ),
        rtt: [RttData::from_nanos(flow.rtt_nanos), RttData::from_nanos(0)],
        // The bridge wire carries no retransmit timestamps (counts only).
        retransmit_times_down: vec![],
        retransmit_times_up: vec![],
        total_bytes: flow.bytes,
        protocol: flow.protocol.clone(),
        circuit_id,
        circuit_name,
        remote_ip: flow.flow_id.remote_ip.to_string(),
    }
}

fn flow_timeline_boot_time() -> Result<u64, TimeError> {
    flow_timeline_boot_time_from(|| time_since_boot().map(Duration::from), unix_now)
}

fn flow_timeline_boot_time_from(
    since_boot: impl FnOnce() -> Result<Duration, TimeError>,
    now: impl FnOnce() -> Result<u64, TimeError>,
) -> Result<u64, TimeError> {
    let since_boot = since_boot()?;
    Ok(now()?.saturating_sub(since_boot.as_secs()))
}

fn all_flows_to_transport(
    boot_time: u64,
    all_flows_for_asn: Vec<(FlowbeeKey, FlowbeeLocalData, FlowAnalysis)>,
) -> Vec<FlowTimeline> {
    all_flows_for_asn
        .iter()
        .filter(|flow| {
            // Total flow time > 2 seconds
            flow.1.last_seen - flow.1.start_time > 2_000_000_000
        })
        .map(|flow| {
            let (circuit_id, mut circuit_name) = resolve_circuit_metadata_for_ip(&flow.0.local_ip);
            if circuit_name.is_empty() {
                circuit_name = flow.0.local_ip.as_ip().to_string();
            }

            let retransmit_times_down =
                retry_times_to_unix_seconds(flow.1.get_retry_times_down(), |timestamp| {
                    boot_time + Duration::from_nanos(timestamp).as_secs()
                });
            let retransmit_times_up =
                retry_times_to_unix_seconds(flow.1.get_retry_times_up(), |timestamp| {
                    boot_time + Duration::from_nanos(timestamp).as_secs()
                });

            FlowTimeline {
                start: boot_time + Duration::from_nanos(flow.1.start_time).as_secs(),
                end: boot_time + Duration::from_nanos(flow.1.last_seen).as_secs(),
                duration_nanos: flow.1.last_seen - flow.1.start_time,
                tcp_retransmits: flow.1.tcp_retransmits,
                throughput: vec![],
                rtt: flow.1.get_rtt_array(),
                retransmit_times_down,
                retransmit_times_up,
                total_bytes: flow.1.bytes_sent,
                protocol: flow.2.protocol_analysis.to_string(),
                circuit_id,
                circuit_name,
                remote_ip: flow.0.remote_ip.as_ip().to_string(),
            }
        })
        .collect::<Vec<_>>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::throughput_tracker::flow_data::FlowbeeEffectiveDirection;
    use lqos_sys::flowbee_data::FlowbeeData;
    use lqos_utils::{XdpIpAddress, units::DownUpOrder};

    #[test]
    fn boot_time_helper_propagates_clock_errors() {
        let result = flow_timeline_boot_time_from(|| Err(TimeError::ClockNotReady), || Ok(1_000));

        assert!(matches!(result, Err(TimeError::ClockNotReady)));
    }

    #[test]
    fn boot_time_helper_saturates_underflow() {
        let boot_time = flow_timeline_boot_time_from(|| Ok(Duration::from_secs(20)), || Ok(10))
            .expect("test clock values should build a boot time");

        assert_eq!(boot_time, 0);
    }

    #[test]
    fn flow_timeline_transport_converts_retry_times_to_unix_seconds() {
        let mut key = FlowbeeKey::default();
        key.local_ip = XdpIpAddress::from_ip("192.0.2.10".parse().expect("test IP should parse"));
        key.remote_ip =
            XdpIpAddress::from_ip("198.51.100.10".parse().expect("test IP should parse"));
        key.ip_protocol = 6;
        let mut raw = FlowbeeData::default();
        raw.start_time = 1_000_000_000;
        raw.last_seen = 5_000_000_000;
        raw.bytes_sent = DownUpOrder::new(1_000, 2_000);
        raw.tcp_retransmits = DownUpOrder::new(2, 1);

        let mut local = FlowbeeLocalData::from_flow(&raw, &key);
        local.record_tcp_retry_time(FlowbeeEffectiveDirection::Download, 0);
        local.record_tcp_retry_time(FlowbeeEffectiveDirection::Download, 2_000_000_000);
        local.record_tcp_retry_time(FlowbeeEffectiveDirection::Download, 3_000_000_000);
        local.record_tcp_retry_time(FlowbeeEffectiveDirection::Upload, 4_000_000_000);
        let analysis = FlowAnalysis::new(&key);

        let rows = all_flows_to_transport(1_700_000_000, vec![(key, local, analysis)]);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].retransmit_times_down, vec![1_700_000_002, 1_700_000_003]);
        assert_eq!(rows[0].retransmit_times_up, vec![1_700_000_004]);
    }

    fn flow_row(flow_hash: u64, down: u64, up: u64) -> analytics_ipc::FlowRow {
        analytics_ipc::FlowRow {
            id: analytics_ipc::FlowId {
                circuit_id: 1,
                flow_hash,
                subscriber_ip: "192.0.2.1".parse().expect("test IP"),
                remote_ip: "198.51.100.1".parse().expect("test IP"),
                subscriber_port: 12_345,
                remote_port: 443,
                protocol: 6,
            },
 // 3b sweep-fixture guard: demand != delivered.
            actual: analytics_ipc::DirectionalCounters {
                download_bytes: down + 10_000,
                upload_bytes: up + 10_000,
            },
            shaped: analytics_ipc::DirectionalCounters {
                download_bytes: down,
                upload_bytes: up,
            },
            rtt_samples_us: Vec::new(),
            tcp_sent_packets: 0,
            tcp_retransmits: 0,
            first_seen_millis: 0,
            last_seen_millis: 1,
            state: analytics_ipc::FlowState::Active,
            ..analytics_ipc::FlowRow::default()
        }
    }

    #[test]
    fn history_timeline_builds_rows_and_buckets() {
        let mut history = flow_history::FlowHistory::new();
        let mut snapshot = analytics_ipc::AnalyticsSnapshot::default();
        snapshot.active_flows.push(flow_row(1, 0, 0));
        // Seed the flow, then cross a rollup boundary with a delta.
        history.fold(&snapshot, 10_000, |_| Default::default());
        snapshot.active_flows[0] = flow_row(1, 1_000, 100);
        history.fold(&snapshot, 70_000, |_| Default::default());
        // A finished flow in PART A, resolvable to the ASN.
        snapshot.recent_flows.push(analytics_ipc::FlowRow {
            state: analytics_ipc::FlowState::Fin,
            // Deliverd != demand: `RecentFlow.bytes` is asserted from `shaped`.
            actual: analytics_ipc::DirectionalCounters {
                download_bytes: 5_000,
                upload_bytes: 500,
            },
            shaped: analytics_ipc::DirectionalCounters {
                download_bytes: 2_000,
                upload_bytes: 200,
            },
            first_seen_millis: 0,
            last_seen_millis: 5_000,
            ..flow_row(2, 0, 0)
        });
        history.fold(&snapshot, 130_000, |_| Default::default());
        let published = history.publish();

        let rows = timeline_from_history(&published, 1_000, |_| true);
        assert_eq!(rows.len(), 1, "the finished flow is a timeline row");
        assert_eq!(rows[0].total_bytes, DownUpOrder::new(2_000, 200));
        assert_eq!(rows[0].start, 1_000);

        let buckets = published.asn_buckets(None, 10);
        assert!(
            buckets.iter().any(|bucket| bucket.bytes.down == 1_000),
            "the delta landed in a committed bucket"
        );
    }
}
