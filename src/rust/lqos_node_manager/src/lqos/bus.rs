use lqos_bus::{bus_request, BusRequest, BusResponse, FlowTransport, IpStats, QueueStoreTransit, TcHandle};
use lqos_config::{NetworkJsonTransport, Tunables};
use serde::{Serialize, Deserialize};
use axum::Json;
use std::net::IpAddr;

pub async fn watch_queue(circuit_id: String) {
    bus_request(vec![BusRequest::WatchQueue(circuit_id)]).await.unwrap();
}

pub async fn update_lqos_d_tuning(period: u64, tuning: Json<Tunables>) {
    bus_request(vec![BusRequest::UpdateLqosDTuning(period, (*tuning).clone())])
        .await
        .unwrap();
}

#[derive(Serialize, Clone, Default)]
pub struct LqosStats {
    pub bus_requests_since_start: u64,
    pub time_to_poll_hosts_us: u64,
    pub high_watermark: (u64, u64),
    pub tracked_flows: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct IpStatsWithPlan {
    pub ip_address: String,
    pub bits_per_second: (u64, u64),
    pub packets_per_second: (u64, u64),
    pub median_tcp_rtt: f32,
    pub tc_handle: TcHandle,
    pub circuit_id: String,
    pub plan: (u32, u32),
}

impl From<&IpStats> for IpStatsWithPlan {
    fn from(i: &IpStats) -> Self {
        let mut result = Self {
            ip_address: i.ip_address.clone(),
            bits_per_second: i.bits_per_second,
            packets_per_second: i.packets_per_second,
            median_tcp_rtt: i.median_tcp_rtt,
            tc_handle: i.tc_handle,
            circuit_id: i.circuit_id.clone(),
            plan: (0, 0),
        };
        if !result.circuit_id.is_empty() {
        if let Some(circuit) = SHAPED_DEVICES
            .read()
            .unwrap()
            .devices
            .iter()
            .find(|sd| sd.circuit_id == result.circuit_id)
        {
            let name = if circuit.circuit_name.len() > 20 {
                &circuit.circuit_name[0..20]
            } else {
                &circuit.circuit_name
            };
            result.ip_address = format!("{} ({})", name, result.ip_address);
            result.plan = (circuit.download_max_mbps, circuit.download_min_mbps);
        }
        }
        result
    }
}

pub async fn get_lqos_stats() -> LqosStats {
    for msg in bus_request(vec![BusRequest::GetLqosStats]).await.unwrap() {
        if let BusResponse::LqosdStats { bus_requests, time_to_poll_hosts, high_watermark, tracked_flows } = msg {
            return LqosStats {
                bus_requests_since_start: bus_requests,
                time_to_poll_hosts_us: time_to_poll_hosts,
                high_watermark,
                tracked_flows,
            };
        }
    }
    LqosStats::default()
}

pub async fn all_unknown_ips() -> Vec<IpStats> {
    if let Ok(messages) = bus_request(vec![BusRequest::AllUnknownIps]).await {
        for msg in messages {
            if let BusResponse::AllUnknownIps(unknowns) = msg {
                let result: Vec<IpStats> = unknowns
                .iter()
                .filter(|ip| {
                    if let Ok(ip) = ip.ip_address.parse::<IpAddr>() {
                        let lookup = match ip {
                            IpAddr::V4(ip) => ip.to_ipv6_mapped(),
                            IpAddr::V6(ip) => ip,
                        };
                        SHAPED_DEVICES.read().unwrap().trie.longest_match(lookup).is_none()
                    } else {
                        false
                    }
                })
                .cloned()
                .collect();
                return result
            }
        }
    }
    Vec::new()
}

pub async fn get_worst_rtt(start: u32, end: u32) -> Vec<IpStatsWithPlan> {
    if let Ok(messages) = bus_request(vec![BusRequest::GetWorstRtt { start: start, end: end }]).await {
        for msg in messages {
            if let BusResponse::WorstRtt(stats) = msg {
                return stats.iter().map(|tt| tt.into()).collect();
            }
        }
    }
    Vec::new()
}

pub async fn get_top_downloaders() -> Vec<IpStatsWithPlan> {
    if let Ok(messages) = bus_request(vec![BusRequest::GetTopNDownloaders { start: 0, end: 10 }]).await {
        for msg in messages {
            if let BusResponse::TopDownloaders(stats) = msg {
                return stats.iter().map(|tt| tt.into()).collect();
            }
        }
    }
    Vec::new()
}

pub async fn rtt_histogram() -> Vec<u32> {
    if let Ok(messages) = bus_request(vec![BusRequest::RttHistogram]).await {
        for msg in messages {
            if let BusResponse::RttHistogram(stats) = msg {
                return stats
            }
        }
    }
    Vec::new()
}

pub async fn get_raw_queue_data(circuit_id: String) -> QueueStoreTransit {
    let responses = bus_request(vec![BusRequest::GetRawQueueData(circuit_id)]).await.unwrap();
    let result = match &responses[0] {
        BusResponse::RawQueueData(Some(msg)) => {
            *msg.clone()
        }
        _ => QueueStoreTransit::default()
    };
    result
}

pub async fn top_map_queues(n_queues: usize) -> Vec<(usize, NetworkJsonTransport)> {
    if let Ok(responses) = bus_request(vec![BusRequest::TopMapQueues(n_queues)]).await {
        for response in responses {
            if let BusResponse::NetworkMap(nodes) = response {
                return nodes.to_owned()
            }
        }
    }
    Vec::new()
}

pub async fn get_flow_stats(ip_list: String) -> Vec<(FlowTransport, Option<FlowTransport>)> {
    let mut result = Vec::new();
    let request: Vec<BusRequest> = ip_list.split(',').map(|ip| BusRequest::GetFlowStats(ip.to_string())).collect();
    let responses = bus_request(request).await.unwrap();
    for r in responses.iter() {
        if let BusResponse::FlowData(flow) = r {
            result.extend_from_slice(flow);
        }
    }
    result
}