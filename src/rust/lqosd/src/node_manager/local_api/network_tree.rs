use crate::shaped_devices_tracker::NETWORK_JSON;
use lqos_config::NetworkJsonTransport;

pub fn network_tree_data() -> Vec<(usize, NetworkJsonTransport)> {
    let net_json = NETWORK_JSON.read();
    net_json
        .get_nodes_when_ready()
        .iter()
        .enumerate()
        .map(|(i, n)| (i, n.clone_to_transit()))
        .collect()
}
