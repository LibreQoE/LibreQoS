use crate::shaped_devices_tracker::NETWORK_JSON;
use axum::Json;
use lqos_config::NetworkJsonTransport;

pub async fn get_network_tree() -> Json<Vec<(usize, NetworkJsonTransport)>> {
    let net_json = NETWORK_JSON.read().unwrap();
    let result: Vec<(usize, NetworkJsonTransport)> = net_json
        .get_nodes_when_ready()
        .iter()
        .enumerate()
        .map(|(i, n)| (i, n.clone_to_transit()))
        .collect();

    Json(result)
}
