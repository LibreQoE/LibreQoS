use crate::shaped_devices_tracker::full_network_map_snapshot;
use lqos_config::NetworkJsonTransport;

pub fn network_tree_data() -> Vec<(usize, NetworkJsonTransport)> {
    full_network_map_snapshot()
}
