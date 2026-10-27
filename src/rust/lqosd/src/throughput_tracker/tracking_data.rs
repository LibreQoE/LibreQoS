use super::throughput_entry::ThroughputEntry;
use crate::throughput_tracker::resolve_flow_device;
use fxhash::FxHashMap;
use lqos_config::NetworkJson;
use lqos_utils::XdpIpAddress;
use lqos_utils::{
    qoq_heatmap::TemporalQoqHeatmap,
    temporal_heatmap::TemporalHeatmap,
    units::{AtomicDownUp, DownUpOrder},
};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use tracing::info;

pub const MAX_RETRY_TIMES: usize = 128;

pub struct ThroughputTracker {
    pub(crate) cycle: AtomicU64,
    pub(crate) raw_data: Mutex<HashMap<XdpIpAddress, ThroughputEntry>>,
    pub(crate) actual_bytes_per_second: AtomicDownUp,
    pub(crate) packets_per_second: AtomicDownUp,
    pub(crate) tcp_packets_per_second: AtomicDownUp,
    pub(crate) udp_packets_per_second: AtomicDownUp,
    pub(crate) icmp_packets_per_second: AtomicDownUp,
    pub(crate) shaped_actual_bytes_per_second: AtomicDownUp,
    pub(crate) circuit_heatmaps: Mutex<FxHashMap<i64, TemporalHeatmap>>,
    pub(crate) circuit_qoq_heatmaps: Mutex<FxHashMap<i64, TemporalQoqHeatmap>>,
    pub(crate) global_heatmap: Mutex<TemporalHeatmap>,
    pub(crate) global_qoq_heatmap: Mutex<TemporalQoqHeatmap>,
}

impl ThroughputTracker {
    pub(crate) fn new() -> Self {
        // The capacity used to be taken from MAX_TRACKED_IPs, but
        // that's quite wasteful for smaller systems. So we're starting
        // small and allowing vector growth. That will slow down the
        // first few cycles, but it should be fine after that.
        Self {
            cycle: AtomicU64::new(super::RETIRE_AFTER_SECONDS),
            raw_data: Mutex::default(),
            actual_bytes_per_second: AtomicDownUp::zeroed(),
            packets_per_second: AtomicDownUp::zeroed(),
            tcp_packets_per_second: AtomicDownUp::zeroed(),
            udp_packets_per_second: AtomicDownUp::zeroed(),
            icmp_packets_per_second: AtomicDownUp::zeroed(),
            shaped_actual_bytes_per_second: AtomicDownUp::zeroed(),
            circuit_heatmaps: Mutex::default(),
            circuit_qoq_heatmaps: Mutex::default(),
            global_heatmap: Mutex::new(TemporalHeatmap::new()),
            global_qoq_heatmap: Mutex::new(TemporalQoqHeatmap::new()),
        }
    }

    pub(crate) fn refresh_circuit_ids(&self, lock: &NetworkJson) {
        let catalog = lqos_network_devices::network_devices_catalog();
        let mut raw_data = self.raw_data.lock();
        raw_data.iter_mut().for_each(|(ip, data)| {
            let shaped_device =
                resolve_flow_device(&catalog, ip, data.device_hash, data.circuit_hash);
            if data.device_hash.is_none()
                && let Some(device) = shaped_device
            {
                data.device_hash = Some(device.device_hash);
            }
            if data.circuit_hash.is_none()
                && let Some(device) = shaped_device
            {
                data.circuit_hash = Some(device.circuit_hash);
            }
            if let Some(device) = shaped_device {
                data.circuit_id = Some(device.circuit_id.clone());
                data.network_json_parents = lock.get_parents_for_circuit_id(&device.parent_node);
            } else {
                data.network_json_parents = None;
            }
        });
    }

    pub(crate) fn actual_bits_per_second(&self) -> DownUpOrder<u64> {
        self.actual_bytes_per_second
            .as_down_up()
            .to_bits_from_bytes()
    }

    pub(crate) fn shaped_actual_bits_per_second(&self) -> DownUpOrder<u64> {
        self.shaped_actual_bytes_per_second
            .as_down_up()
            .to_bits_from_bytes()
    }

    pub(crate) fn packets_per_second(&self) -> DownUpOrder<u64> {
        self.packets_per_second.as_down_up()
    }

    pub(crate) fn tcp_packets_per_second(&self) -> DownUpOrder<u64> {
        self.tcp_packets_per_second.as_down_up()
    }

    pub(crate) fn udp_packets_per_second(&self) -> DownUpOrder<u64> {
        self.udp_packets_per_second.as_down_up()
    }

    pub(crate) fn icmp_packets_per_second(&self) -> DownUpOrder<u64> {
        self.icmp_packets_per_second.as_down_up()
    }

    #[allow(dead_code)]
    pub(crate) fn dump(&self) {
        let raw_data = self.raw_data.lock();
        for (k, v) in raw_data.iter() {
            let ip = k.as_ip();
            info!("{:<34}{:?}", ip, v.tc_handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::throughput_tracker::resolve_flow_device;
    use lqos_config::{ConfigShapedDevices, ShapedDevice};
    use lqos_network_devices::{NetworkDevicesCatalog, ShapedDevicesCatalog};
    use lqos_utils::XdpIpAddress;
    use lqos_utils::hash_to_i64;
    use std::net::Ipv4Addr;
    use std::sync::Arc;

    #[test]
    fn shaped_device_lookup_falls_back_to_ip_when_hashes_missing() {
        let mut shaped = ConfigShapedDevices::default();
        shaped.replace_with_new_data(vec![ShapedDevice {
            circuit_id: "circuit-1".to_string(),
            device_id: "device-1".to_string(),
            parent_node: "Parent-A".to_string(),
            ipv4: vec![(Ipv4Addr::new(192, 168, 1, 10), 32)],
            ..Default::default()
        }]);
        let shaped_catalog = ShapedDevicesCatalog::from_shaped_devices(Arc::new(shaped));
        let catalog = NetworkDevicesCatalog::from_snapshots(shaped_catalog, Arc::new(Vec::new()));
        let ip = XdpIpAddress::from_ip("192.168.1.10".parse().expect("test IP should parse"));

        let matched =
            resolve_flow_device(&catalog, &ip, None, None)
                .expect("lookup should resolve by IP");

        assert_eq!(matched.circuit_hash, hash_to_i64("circuit-1"));
        assert_eq!(matched.device_hash, hash_to_i64("device-1"));
    }
}
