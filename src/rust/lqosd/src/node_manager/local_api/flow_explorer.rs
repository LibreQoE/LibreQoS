use std::time::Duration;
use axum::extract::Path;
use axum::Json;
use serde::Serialize;
use lqos_utils::units::DownUpOrder;
use lqos_utils::unix_time::{time_since_boot, unix_now};
use crate::throughput_tracker::flow_data::{AsnListEntry, RECENT_FLOWS, RttData};

pub async fn asn_list() -> Json<Vec<AsnListEntry>> {
    Json(RECENT_FLOWS.asn_list())
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
}

pub async fn flow_timeline(Path(asn_id): Path<u32>) -> Json<Vec<FlowTimeline>> {
    let time_since_boot = time_since_boot().unwrap();
    let since_boot = Duration::from(time_since_boot);
    let boot_time = unix_now().unwrap() - since_boot.as_secs();

    let all_flows_for_asn = RECENT_FLOWS.all_flows_for_asn(asn_id);

    let flows = all_flows_for_asn
        .iter()
        .filter(|flow| {
            // Total flow time > 2 seconds
            flow.1.last_seen - flow.1.start_time > 2_000_000_000
        })
        .map(|flow| {

            FlowTimeline {
                start: boot_time + Duration::from_nanos(flow.1.start_time).as_secs(),
                end: boot_time + Duration::from_nanos(flow.1.last_seen).as_secs(),
                duration_nanos: flow.1.last_seen - flow.1.start_time,
                tcp_retransmits: flow.1.tcp_retransmits.clone(),
                throughput: flow.1.throughput_buffer.clone(),
                rtt: flow.1.rtt.clone(),
                retransmit_times_down: flow.1.retry_times_down
                    .iter()
                    .map(|t| boot_time + *t)
                    .collect(),
                retransmit_times_up: flow.1.retry_times_up
                    .iter()
                    .map(|t| boot_time + *t)
                    .collect(),
                total_bytes: flow.1.bytes_sent.clone(),
            }
        })
        .collect::<Vec<_>>();

    Json(flows)
}