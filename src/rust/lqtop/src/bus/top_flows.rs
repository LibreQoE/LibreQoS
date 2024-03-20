use lqos_bus::{BusResponse, FlowbeeSummaryData};
use once_cell::sync::Lazy;
use std::sync::Mutex;

pub static TOP_FLOWS: Lazy<Mutex<Vec<FlowbeeSummaryData>>> = Lazy::new(|| Mutex::new(Vec::new()));

pub async fn top_flows(response: &BusResponse) {
    if let BusResponse::TopFlows(stats) = response {
        let mut top_hosts = TOP_FLOWS.lock().unwrap();
        *top_hosts = stats.clone();
    }
}