use crate::throughput_tracker::flow_data::{
    AsnCountryListEntry, AsnListEntry, AsnProtocolListEntry, FlowAnalysis, FlowbeeLocalData,
    RECENT_FLOWS, RttData,
};
use crate::{
    shaped_devices_tracker::{SHAPED_DEVICE_HASH_CACHE, SHAPED_DEVICES},
    throughput_tracker::THROUGHPUT_TRACKER,
};
use lqos_sys::flowbee_data::FlowbeeKey;
use lqos_utils::units::DownUpOrder;
use lqos_utils::unix_time::{time_since_boot, unix_now};
use serde::Serialize;
use std::time::Duration;

pub fn asn_list_data() -> Vec<AsnListEntry> {
    RECENT_FLOWS.asn_list()
}

pub fn country_list_data() -> Vec<AsnCountryListEntry> {
    RECENT_FLOWS.country_list()
}

pub fn protocol_list_data() -> Vec<AsnProtocolListEntry> {
    RECENT_FLOWS.protocol_list()
}

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

pub fn flow_timeline_data(asn_id: u32) -> Vec<FlowTimeline> {
    let time_since_boot = time_since_boot().expect("failed to retrieve time since boot");
    let since_boot = Duration::from(time_since_boot);
    let boot_time = unix_now()
        .expect("failed to retrieve current unix time")
        .saturating_sub(since_boot.as_secs());

    let all_flows_for_asn = RECENT_FLOWS.all_flows_for_asn(asn_id);

    let flows = all_flows_to_transport(boot_time, all_flows_for_asn);

    flows
}

fn all_flows_to_transport(
    boot_time: u64,
    all_flows_for_asn: Vec<(FlowbeeKey, FlowbeeLocalData, FlowAnalysis)>,
) -> Vec<FlowTimeline> {
    let shaped = SHAPED_DEVICES.load();
    let shaped_cache = SHAPED_DEVICE_HASH_CACHE.load();
    let throughput = THROUGHPUT_TRACKER.raw_data.lock();
    all_flows_for_asn
        .iter()
        .filter(|flow| {
            // Total flow time > 2 seconds
            flow.1.last_seen - flow.1.start_time > 2_000_000_000
        })
        .map(|flow| {
            let mut circuit_id = String::new();
            let mut circuit_name = String::new();
            if let Some(te) = throughput.get(&flow.0.local_ip) {
                if let Some(id) = &te.circuit_id {
                    circuit_id = id.clone();
                }
                let shaped_device = te
                    .device_hash
                    .and_then(|hash| shaped_cache.index_by_device_hash(&shaped, hash))
                    .or_else(|| {
                        te.circuit_hash
                            .and_then(|hash| shaped_cache.index_by_circuit_hash(&shaped, hash))
                    })
                    .and_then(|idx| shaped.devices.get(idx));
                if let Some(device) = shaped_device {
                    if circuit_id.is_empty() {
                        circuit_id = device.circuit_id.clone();
                    }
                    circuit_name = device.circuit_name.clone();
                }
            }
            if circuit_name.is_empty() {
                circuit_name = flow.0.local_ip.as_ip().to_string();
            }

            let retransmit_times_down = flow
                .1
                .get_retry_times_down()
                .iter()
                .filter(|n| **n > 0)
                .map(|t| boot_time + Duration::from_nanos(*t).as_secs())
                .collect();
            let retransmit_times_up = flow
                .1
                .get_retry_times_up()
                .iter()
                .filter(|n| **n > 0)
                .map(|t| boot_time + Duration::from_nanos(*t).as_secs())
                .collect();

            FlowTimeline {
                start: boot_time + Duration::from_nanos(flow.1.start_time).as_secs(),
                end: boot_time + Duration::from_nanos(flow.1.last_seen).as_secs(),
                duration_nanos: flow.1.last_seen - flow.1.start_time,
                tcp_retransmits: flow.1.tcp_retransmits.clone(),
                throughput: vec![],
                rtt: flow.1.get_rtt_array(),
                retransmit_times_down,
                retransmit_times_up,
                total_bytes: flow.1.bytes_sent.clone(),
                protocol: flow.2.protocol_analysis.to_string(),
                circuit_id,
                circuit_name,
                remote_ip: flow.0.remote_ip.as_ip().to_string(),
            }
        })
        .collect::<Vec<_>>()
}

pub fn country_timeline_data(iso_code: &str) -> Vec<FlowTimeline> {
    let time_since_boot = time_since_boot().expect("failed to retrieve time since boot");
    let since_boot = Duration::from(time_since_boot);
    let boot_time = unix_now()
        .expect("failed to retrieve current unix time")
        .saturating_sub(since_boot.as_secs());

    let all_flows_for_asn = RECENT_FLOWS.all_flows_for_country(iso_code);

    let flows = all_flows_to_transport(boot_time, all_flows_for_asn);

    flows
}

pub fn protocol_timeline_data(protocol_name: &str) -> Vec<FlowTimeline> {
    let protocol_name = protocol_name.replace("_", "/");
    let time_since_boot = time_since_boot().expect("failed to retrieve time since boot");
    let since_boot = Duration::from(time_since_boot);
    let boot_time = unix_now()
        .expect("failed to retrieve current unix time")
        .saturating_sub(since_boot.as_secs());

    let all_flows_for_asn = RECENT_FLOWS.all_flows_for_protocol(&protocol_name);

    let flows = all_flows_to_transport(boot_time, all_flows_for_asn);

    flows
}
