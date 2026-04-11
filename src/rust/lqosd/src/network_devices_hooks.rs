use crate::shaped_devices_tracker;
use crate::throughput_tracker::THROUGHPUT_TRACKER;
use tracing::warn;

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

    fn on_dynamic_circuits_expired(&self, circuit_ids: &[String]) {
        let Some(sender) = lqos_bakery::BAKERY_SENDER.get() else {
            return;
        };

        for circuit_id in circuit_ids {
            let result = sender.send(lqos_bakery::BakeryCommands::RemoveDynamicCircuitOverlay {
                circuit_id: circuit_id.clone(),
                reply: None,
            });
            if let Err(err) = result {
                warn!(
                    "Unable to enqueue dynamic circuit overlay removal for '{circuit_id}': {err}"
                );
            }
        }
    }
}
