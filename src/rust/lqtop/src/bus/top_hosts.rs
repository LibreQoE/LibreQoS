use lqos_bus::{BusResponse, IpStats};
use once_cell::sync::Lazy;
use std::sync::Mutex;

pub static TOP_HOSTS: Lazy<Mutex<Vec<IpStats>>> = Lazy::new(|| Mutex::new(Vec::new()));

pub async fn top_n(response: &BusResponse) {
    if let BusResponse::TopDownloaders(stats) = response {
        let mut top_hosts = TOP_HOSTS.lock().unwrap();
        *top_hosts = stats.clone();
    }
}