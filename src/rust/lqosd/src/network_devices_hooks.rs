use crate::shaped_devices_tracker;
use crate::throughput_tracker::THROUGHPUT_TRACKER;

pub(crate) struct LqosdNetworkDevicesHooks;

impl lqos_network_devices::DaemonHooks for LqosdNetworkDevicesHooks {
    fn on_shaped_devices_updated(&self) {
        shaped_devices_tracker::invalidate_circuit_live_snapshot();
        shaped_devices_tracker::invalidate_executive_cache_snapshot();
        lqos_network_devices::with_network_json_read(|net_json| {
            THROUGHPUT_TRACKER.refresh_circuit_ids(net_json);
        });
    }

    fn on_network_json_updated(&self) {
        shaped_devices_tracker::invalidate_circuit_live_snapshot();
        shaped_devices_tracker::invalidate_executive_cache_snapshot();
        lqos_network_devices::with_network_json_read(|net_json| {
            THROUGHPUT_TRACKER.refresh_circuit_ids(net_json);
        });
    }
}

