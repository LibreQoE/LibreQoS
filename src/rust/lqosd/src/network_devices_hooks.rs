use crate::shaped_devices_tracker;
use crate::throughput_tracker::THROUGHPUT_TRACKER;
use lqos_bus::TcHandle;
use lqos_config::ShapedDevice;
use std::collections::HashSet;
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tracing::warn;

pub(crate) struct LqosdNetworkDevicesHooks;

const UNKNOWN_IP_PROMOTION_REPLY_TIMEOUT: Duration = Duration::from_secs(60);

static UNKNOWN_IP_PROMOTION_SENDER: OnceLock<mpsc::Sender<ShapedDevice>> = OnceLock::new();
static UNKNOWN_IP_PROMOTIONS_PENDING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn pending_unknown_ip_promotions() -> &'static Mutex<HashSet<String>> {
    UNKNOWN_IP_PROMOTIONS_PENDING.get_or_init(|| Mutex::new(HashSet::new()))
}

fn with_pending_unknown_ip_promotions<R>(f: impl FnOnce(&mut HashSet<String>) -> R) -> R {
    match pending_unknown_ip_promotions().lock() {
        Ok(mut pending) => f(&mut pending),
        Err(poisoned) => {
            let mut pending = poisoned.into_inner();
            f(&mut pending)
        }
    }
}

fn mark_unknown_ip_promotion_pending(circuit_id: &str) -> bool {
    with_pending_unknown_ip_promotions(|pending| pending.insert(circuit_id.to_string()))
}

fn clear_unknown_ip_promotion_pending(circuit_id: &str) {
    with_pending_unknown_ip_promotions(|pending| {
        pending.remove(circuit_id);
    });
}

fn unknown_ip_promotion_sender() -> Result<mpsc::Sender<ShapedDevice>, String> {
    if let Some(sender) = UNKNOWN_IP_PROMOTION_SENDER.get() {
        return Ok(sender.clone());
    }

    let (tx, rx) = mpsc::channel::<ShapedDevice>();
    std::thread::Builder::new()
        .name("dyn-circuit-promoter".to_string())
        .spawn(move || unknown_ip_promotion_worker(rx))
        .map_err(|err| err.to_string())?;

    if UNKNOWN_IP_PROMOTION_SENDER.set(tx.clone()).is_err() {
        return UNKNOWN_IP_PROMOTION_SENDER
            .get()
            .cloned()
            .ok_or_else(|| "unknown IP promotion worker initialized without sender".to_string());
    }

    Ok(tx)
}

fn unknown_ip_promotion_worker(rx: mpsc::Receiver<ShapedDevice>) {
    while let Ok(shaped_device) = rx.recv() {
        let circuit_id = shaped_device.circuit_id.clone();
        apply_unknown_ip_promotion(shaped_device);
        clear_unknown_ip_promotion_pending(&circuit_id);
    }
}

fn apply_unknown_ip_promotion(shaped_device: ShapedDevice) {
    let Some(sender) = lqos_bakery::BAKERY_SENDER.get() else {
        return;
    };

    let (tx, rx) = mpsc::channel::<Result<Option<TcHandle>, String>>();
    if let Err(err) = sender.send(lqos_bakery::BakeryCommands::UpsertDynamicCircuitOverlay {
        shaped_device: Box::new(shaped_device.clone()),
        reply: Some(tx),
    }) {
        warn!(
            "Unable to enqueue dynamic circuit overlay for unknown IP promotion '{}': {err}",
            shaped_device.circuit_id
        );
        return;
    }

    let handle = match rx.recv_timeout(UNKNOWN_IP_PROMOTION_REPLY_TIMEOUT) {
        Ok(Ok(Some(handle))) => handle,
        Ok(Ok(None)) => {
            // Bakery accepted the overlay but could not yet allocate a concrete class handle
            // (e.g., baseline not ready). We'll retry on future observations.
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

    let cpu_count = lqos_sys::num_possible_cpus().map(|n| n.max(1)).unwrap_or(1);
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
}

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

        if !mark_unknown_ip_promotion_pending(&circuit_id) {
            return;
        }

        let sender = match unknown_ip_promotion_sender() {
            Ok(sender) => sender,
            Err(err) => {
                clear_unknown_ip_promotion_pending(&circuit_id);
                warn!("Unable to start unknown IP promotion worker for '{circuit_id}': {err}");
                return;
            }
        };

        if let Err(err) = sender.send(shaped_device) {
            clear_unknown_ip_promotion_pending(&circuit_id);
            warn!("Unable to enqueue unknown IP promotion for '{circuit_id}': {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{clear_unknown_ip_promotion_pending, mark_unknown_ip_promotion_pending};

    #[test]
    fn unknown_ip_promotion_pending_set_deduplicates_until_cleared() {
        let circuit_id = "[dyn] test pending dedupe";
        clear_unknown_ip_promotion_pending(circuit_id);

        assert!(mark_unknown_ip_promotion_pending(circuit_id));
        assert!(!mark_unknown_ip_promotion_pending(circuit_id));

        clear_unknown_ip_promotion_pending(circuit_id);
        assert!(mark_unknown_ip_promotion_pending(circuit_id));

        clear_unknown_ip_promotion_pending(circuit_id);
    }
}
