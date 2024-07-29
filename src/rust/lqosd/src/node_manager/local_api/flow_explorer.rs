use std::time::Duration;
use axum::extract::Path;
use axum::Json;
use serde::Serialize;
use lqos_sys::flowbee_data::FlowbeeKey;
use lqos_utils::units::DownUpOrder;
use lqos_utils::unix_time::{time_since_boot, unix_now};
use crate::shaped_devices_tracker::SHAPED_DEVICES;
use crate::throughput_tracker::flow_data::{AsnListEntry, AsnCountryListEntry, AsnProtocolListEntry,
   RECENT_FLOWS, RttData, FlowbeeLocalData, FlowAnalysis};

pub async fn asn_list() -> Json<Vec<AsnListEntry>> {
    Json(RECENT_FLOWS.asn_list())
}

pub async fn country_list() -> Json<Vec<AsnCountryListEntry>> {
    Json(RECENT_FLOWS.country_list())
}

pub async fn protocol_list() -> Json<Vec<AsnProtocolListEntry>> {
    Json(RECENT_FLOWS.protocol_list())
}

#[derive(Serialize)]
pub struct FlowTimeline {
    start: u64,
    end: u64,
    duration_nanos: u64,
    throughput: Vec<DownUpOrder<u64>>,
    tcp_retransmits: DownUpOrder<u16>,
    rtt: [RttData; 2],
    retransmit_times_down: Vec<u64>,
    retransmit_times_up: Vec<u64>,
    total_bytes: DownUpOrder<u64>,
    protocol: String,
    circuit_id: String,
    circuit_name: String,
}

pub async fn flow_timeline(Path(asn_id): Path<u32>) -> Json<Vec<FlowTimeline>> {
    let time_since_boot = time_since_boot().unwrap();
    let since_boot = Duration::from(time_since_boot);
    let boot_time = unix_now().unwrap() - since_boot.as_secs();

    let all_flows_for_asn = RECENT_FLOWS.all_flows_for_asn(asn_id);

    let flows = all_flows_to_transport(boot_time, all_flows_for_asn);

    Json(flows)
}

fn all_flows_to_transport(boot_time: u64, all_flows_for_asn: Vec<(FlowbeeKey, FlowbeeLocalData, FlowAnalysis)>) -> Vec<FlowTimeline> {
    all_flows_for_asn
        .iter()
        .filter(|flow| {
            // Total flow time > 2 seconds
            flow.1.last_seen - flow.1.start_time > 2_000_000_000
        })
        .map(|flow| {
            let (circuit_id, mut circuit_name) = {
                let sd = SHAPED_DEVICES.read().unwrap();
                sd.get_circuit_id_and_name_from_ip(&flow.0.local_ip).unwrap_or((String::new(), String::new()))
            };
            if circuit_name.is_empty() {
                circuit_name = flow.0.local_ip.as_ip().to_string();
            }

            FlowTimeline {
                start: boot_time + Duration::from_nanos(flow.1.start_time).as_secs(),
                end: boot_time + Duration::from_nanos(flow.1.last_seen).as_secs(),
                duration_nanos: flow.1.last_seen - flow.1.start_time,
                tcp_retransmits: flow.1.tcp_retransmits.clone(),
                throughput: flow.1.throughput_buffer.clone(),
                rtt: flow.1.rtt.clone(),
                retransmit_times_down: flow.1.retry_times_down
                    .iter()
                    .map(|t| boot_time + Duration::from_nanos(*t).as_secs())
                    .collect(),
                retransmit_times_up: flow.1.retry_times_up
                    .iter()
                    .map(|t| boot_time + Duration::from_nanos(*t).as_secs())
                    .collect(),
                total_bytes: flow.1.bytes_sent.clone(),
                protocol: flow.2.protocol_analysis.to_string(),
                circuit_id,
                circuit_name,
            }
        })
        .collect::<Vec<_>>()
}

pub async fn country_timeline(Path(country_name): Path<String>) -> Json<Vec<FlowTimeline>> {
    let time_since_boot = time_since_boot().unwrap();
    let since_boot = Duration::from(time_since_boot);
    let boot_time = unix_now().unwrap() - since_boot.as_secs();

    let all_flows_for_asn = RECENT_FLOWS.all_flows_for_country(&country_name);

    let flows = all_flows_to_transport(boot_time, all_flows_for_asn);

    Json(flows)
}

pub async fn protocol_timeline(Path(protocol_name): Path<String>) -> Json<Vec<FlowTimeline>> {
    let protocol_name = protocol_name.replace("_", "/");
    let time_since_boot = time_since_boot().unwrap();
    let since_boot = Duration::from(time_since_boot);
    let boot_time = unix_now().unwrap() - since_boot.as_secs();

    let all_flows_for_asn = RECENT_FLOWS.all_flows_for_protocol(&protocol_name);

    let flows = all_flows_to_transport(boot_time, all_flows_for_asn);

    Json(flows)
}