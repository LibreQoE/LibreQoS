use crate::shaped_devices_tracker::full_network_map_snapshot;
use lqos_config::NetworkJsonTransport;

/// Returns the current full network tree snapshot for websocket/API consumers.
pub fn network_tree_data() -> Vec<(usize, NetworkJsonTransport)> {
    full_network_map_snapshot()
}
