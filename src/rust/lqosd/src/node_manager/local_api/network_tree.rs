use crate::shaped_devices_tracker::{NETWORK_JSON, node_to_transport};
use lqos_config::NetworkJsonTransport;

pub fn network_tree_data() -> Vec<(usize, NetworkJsonTransport)> {
    let net_json = NETWORK_JSON.read();
    net_json
        .get_nodes_when_ready()
        .iter()
        .enumerate()
        .map(|(i, n)| (i, node_to_transport(n)))
        .collect()
}
