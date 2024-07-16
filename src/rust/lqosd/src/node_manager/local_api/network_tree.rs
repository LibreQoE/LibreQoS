use axum::extract::Path;
use axum::Json;
use lqos_config::NetworkJsonTransport;
use crate::shaped_devices_tracker::NETWORK_JSON;

pub async fn get_network_tree(
    Path(parent): Path<usize>
) -> Json<Vec<(usize, NetworkJsonTransport)>> {
    let net_json = NETWORK_JSON.read().unwrap();
    let result: Vec<(usize, NetworkJsonTransport)> = net_json
        .get_nodes_when_ready()
        .iter()
        .enumerate()
        .map(|(i, n) | (i, n.clone_to_transit()))
        .collect();

    Json(result)
}