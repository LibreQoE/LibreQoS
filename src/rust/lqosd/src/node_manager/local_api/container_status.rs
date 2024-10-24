//! Provides a diagnostic API for tracking the status of various
//! data containers. Designed to help identify/diagnose RAM
//! usage issues.

use axum::Json;
use serde::Serialize;
use crate::shaped_devices_tracker::{NETWORK_JSON, SHAPED_DEVICES};
use crate::throughput_tracker::flow_data::{FlowAnalysisSystem, ALL_FLOWS, RECENT_FLOWS};
use crate::throughput_tracker::THROUGHPUT_TRACKER;

#[derive(Debug, Serialize, Default)]
pub struct ContainerSize {
    pub size: usize,
    pub capacity: usize,
}

#[derive(Debug, Serialize)]
pub struct ContainerStatus {
    pub tracked_flows: ContainerSize,
    pub recent_flows: ContainerSize,
    pub throughput_tracker: ContainerSize,
    pub shaped_devices: ContainerSize,
    pub shaped_devices_trie: usize,
    pub asn_trie: usize,
    pub geo_trie: usize,
    pub asn_lookup_table: ContainerSize,
    pub net_json: ContainerSize,
}

fn tracked_flows() -> ContainerSize {
    let all_flows = ALL_FLOWS.lock().unwrap();
    let size = all_flows.len();
    let capacity = all_flows.capacity();
    ContainerSize { size, capacity }
}

fn recent_flows() -> ContainerSize {
    let (size, capacity) = RECENT_FLOWS.len_and_capacity();
    ContainerSize { size, capacity }
}

fn throughput_tracker() -> ContainerSize {
    let raw_data = THROUGHPUT_TRACKER.raw_data.lock().unwrap();
    ContainerSize {
        size: raw_data.len(),
        capacity: raw_data.capacity(),
    }
}

fn shaped_devices() -> (ContainerSize, usize) {
    let sd = SHAPED_DEVICES.read().unwrap();
    let size = sd.devices.len();
    let capacity = sd.devices.capacity();
    let trie_size = sd.trie.len();
    (ContainerSize { size, capacity }, trie_size.0 + trie_size.1)
}

fn asn_analysis() -> (usize, usize, ContainerSize) {
    let (asn_trie, geo_trie, lookup_len, lookup_capacity) = FlowAnalysisSystem::len_and_capacity();
    (
        asn_trie,
        geo_trie,
        ContainerSize { size: lookup_len, capacity: lookup_capacity }
    )
}

fn net_json() -> ContainerSize {
    let nj = NETWORK_JSON.read().unwrap();
    let (size, capacity) = nj.len_and_capacity();
    ContainerSize { size, capacity }
}

pub async fn container_status() -> Json<ContainerStatus> {
    let (shaped_devices, shaped_devices_trie) = shaped_devices();
    let (asn_trie, geo_trie, asn_lookup_table) = asn_analysis();
    Json(ContainerStatus {
        tracked_flows: tracked_flows(),
        recent_flows: recent_flows(),
        throughput_tracker: throughput_tracker(),
        shaped_devices,
        shaped_devices_trie,
        asn_trie,
        geo_trie,
        asn_lookup_table,
        net_json: net_json(),
    })
}