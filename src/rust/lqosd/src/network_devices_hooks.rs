use crate::shaped_devices_tracker;
use crate::throughput_tracker::THROUGHPUT_TRACKER;
use lqos_bus::TcHandle;
use lqos_config::ShapedDevice;
use std::sync::mpsc;
use std::time::Duration;
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

    fn on_unknown_ip_promoted(&self, shaped_device: &ShapedDevice) {
        let shaped_device = shaped_device.clone();
        let circuit_id = shaped_device.circuit_id.clone();

        let result = std::thread::Builder::new()
            .name("dyn-circuit-unknown-ip".to_string())
            .spawn(move || {
                let Some(sender) = lqos_bakery::BAKERY_SENDER.get() else {
                    return;
                };

                let (tx, rx) = mpsc::channel::<Result<Option<TcHandle>, String>>();
                if let Err(err) =
                    sender.send(lqos_bakery::BakeryCommands::UpsertDynamicCircuitOverlay {
                        shaped_device: Box::new(shaped_device.clone()),
                        reply: Some(tx),
                    })
                {
                    warn!(
                        "Unable to enqueue dynamic circuit overlay for unknown IP promotion '{}': {err}",
                        shaped_device.circuit_id
                    );
                    return;
                }

                let handle = match rx.recv_timeout(Duration::from_secs(10)) {
                    Ok(Ok(Some(handle))) => handle,
                    Ok(Ok(None)) => {
                        // Bakery accepted the overlay but could not yet allocate a concrete class
                        // handle (e.g., baseline not ready). We'll retry on future observations.
                        return;
                    }
                    Ok(Err(err)) => {
                        warn!(
                            "Bakery rejected dynamic circuit overlay for unknown IP promotion '{}': {err}",
                            shaped_device.circuit_id
                        );
                        return;
                    }
                    Err(err) => {
                        warn!(
                            "Timeout waiting for Bakery reply while promoting unknown IP '{}': {err}",
                            shaped_device.circuit_id
                        );
                        return;
                    }
                };

                let circuit_hash = if shaped_device.circuit_hash != 0 {
                    shaped_device.circuit_hash
                } else {
                    lqos_utils::hash_to_i64(&shaped_device.circuit_id)
                };
                let device_hash = if shaped_device.device_hash != 0 {
                    shaped_device.device_hash
                } else {
                    lqos_utils::hash_to_i64(&shaped_device.device_id)
                };

                let cpu_count = lqos_sys::num_possible_cpus()
                    .map(|n| n.max(1))
                    .unwrap_or(1);
                let cpu = ((circuit_hash as u64) % (cpu_count as u64)) as u32;

                for (ip, prefix) in shaped_device.ipv4.iter() {
                    let addr = if *prefix == 32 {
                        ip.to_string()
                    } else {
                        format!("{ip}/{prefix}")
                    };
                    if let Err(err) = lqos_sys::add_ip_to_tc(
                        &addr,
                        handle,
                        cpu,
                        false,
                        circuit_hash as u64,
                        device_hash as u64,
                    ) {
                        warn!(
                            "Unable to map unknown IP dynamic circuit '{}' for {addr}: {err:?}",
                            shaped_device.circuit_id
                        );
                    }
                }
                for (ip, prefix) in shaped_device.ipv6.iter() {
                    let addr = if *prefix == 128 {
                        ip.to_string()
                    } else {
                        format!("{ip}/{prefix}")
                    };
                    if let Err(err) = lqos_sys::add_ip_to_tc(
                        &addr,
                        handle,
                        cpu,
                        false,
                        circuit_hash as u64,
                        device_hash as u64,
                    ) {
                        warn!(
                            "Unable to map unknown IP dynamic circuit '{}' for {addr}: {err:?}",
                            shaped_device.circuit_id
                        );
                    }
                }

                if let Err(err) = lqos_sys::clear_hot_cache() {
                    warn!(
                        "Unable to clear hot cache after mapping unknown IP dynamic circuit '{}': {err:?}",
                        shaped_device.circuit_id
                    );
                }
            });

        if let Err(err) = result {
            warn!(
                "Unable to spawn unknown IP promotion helper thread for '{}': {err}",
                circuit_id
            );
        }
    }
}
