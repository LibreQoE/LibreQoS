use lqos_bus::{bus_request, BusRequest, BusResponse};
use lqos_config::Tunables;
use serde::{Serialize};
use axum::Json;

pub async fn watch_queue(circuit_id: String) {
    bus_request(vec![BusRequest::WatchQueue(circuit_id)]).await.unwrap();
}

pub async fn update_tuning(period: u64, tuning: Json<Tunables>) {
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

pub async fn get_stats() -> LqosStats {
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