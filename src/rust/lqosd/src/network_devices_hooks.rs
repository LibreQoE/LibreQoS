use crate::shaped_devices_tracker;
use crate::throughput_tracker::THROUGHPUT_TRACKER;

pub(crate) struct LqosdNetworkDevicesHooks;

/// The two snapshot hooks `lqosd` needs: a new shaped-devices or network.json
/// snapshot invalidates the derived live views and re-resolves the circuit ids
/// the throughput tracker attributes flows to. The dynamic-circuit transitions
/// (promotion, expiry, supersede) need no hook: the actor owns the store, and
/// the topology fold publishes their shaping effects.
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
