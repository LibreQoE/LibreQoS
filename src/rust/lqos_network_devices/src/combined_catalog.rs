use crate::{DynamicCircuit, ShapedDevicesCatalog};
use fxhash::FxHashMap;
use ip_network::IpNetwork;
use ip_network_table::IpNetworkTable;
use lqos_config::ShapedDevice;
use lqos_utils::{
    XdpIpAddress, is_valid_ipv4_prefix, is_valid_ipv6_prefix, normalize_circuit_id_key,
    unique_mapped_circuit_hashes,
};
use std::net::IpAddr;
use std::sync::Arc;

/// Snapshot handle for shaped devices plus runtime dynamic circuits.
///
/// This catalog is intended for read-heavy paths (dashboards, APIs) that need to
/// treat dynamic circuits as first-class circuits alongside `ShapedDevices.csv`.
pub struct NetworkDevicesCatalog {
    shaped: ShapedDevicesCatalog,
    dynamic: Arc<Vec<DynamicCircuit>>,
    dyn_by_device_hash: FxHashMap<i64, usize>,
    dyn_by_circuit_hash: FxHashMap<i64, usize>,
    dyn_by_circuit_id: FxHashMap<String, usize>,
    dyn_ip_table: IpNetworkTable<usize>,
}

impl Clone for NetworkDevicesCatalog {
    fn clone(&self) -> Self {
        Self::from_snapshots(self.shaped.clone(), self.dynamic.clone())
    }
}

impl NetworkDevicesCatalog {
    /// Builds a combined catalog from explicit snapshots.
    pub fn from_snapshots(shaped: ShapedDevicesCatalog, dynamic: Arc<Vec<DynamicCircuit>>) -> Self {
        let mut dyn_by_device_hash = FxHashMap::default();
        let mut dyn_by_circuit_hash = FxHashMap::default();
        let mut dyn_by_circuit_id = FxHashMap::default();
        let mut dyn_ip_table = IpNetworkTable::new();

        for (idx, circuit) in dynamic.iter().enumerate() {
            dyn_by_device_hash.insert(circuit.shaped.device_hash, idx);
            dyn_by_circuit_hash.insert(circuit.shaped.circuit_hash, idx);
            dyn_by_circuit_id.insert(normalize_circuit_id_key(&circuit.shaped.circuit_id), idx);

            for (ipv4, prefix) in &circuit.shaped.ipv4 {
                let prefix = prefix.saturating_add(96).min(128);
                if let Ok(net) = IpNetwork::new(ipv4.to_ipv6_mapped(), prefix as u8) {
                    dyn_ip_table.insert(net, idx);
                }
            }
            for (ipv6, prefix) in &circuit.shaped.ipv6 {
                if *prefix <= 128
                    && let Ok(net) = IpNetwork::new(*ipv6, *prefix as u8)
                {
                    dyn_ip_table.insert(net, idx);
                }
            }
        }

        Self {
            shaped,
            dynamic,
            dyn_by_device_hash,
            dyn_by_circuit_hash,
            dyn_by_circuit_id,
            dyn_ip_table,
        }
    }

    /// Returns the underlying static shaped-devices catalog (`ShapedDevices.csv`).
    pub fn shaped_devices(&self) -> &ShapedDevicesCatalog {
        &self.shaped
    }

    /// Returns the dynamic circuit overlay snapshot.
    pub fn dynamic_circuits(&self) -> &[DynamicCircuit] {
        self.dynamic.as_ref()
    }

    /// Iterates over static shaped-device rows (`ShapedDevices.csv`).
    pub fn iter_static_devices(&self) -> impl Iterator<Item = &ShapedDevice> {
        self.shaped.iter_devices()
    }

    /// Iterates over dynamic circuit overlay entries as shaped-device rows.
    pub fn iter_dynamic_devices(&self) -> impl Iterator<Item = &ShapedDevice> {
        self.dynamic.iter().map(|circuit| &circuit.shaped)
    }

    /// Iterates over both static and dynamic shaped-device rows.
    pub fn iter_all_devices(&self) -> impl Iterator<Item = &ShapedDevice> {
        self.iter_static_devices()
            .chain(self.iter_dynamic_devices())
    }

    /// Returns the longest-prefix match entry for an IP address.
    ///
    /// This prefers static shaped devices, then falls back to dynamic circuits.
    pub fn device_longest_match_for_ip(
        &self,
        ip: &XdpIpAddress,
    ) -> Option<(IpNetwork, &ShapedDevice)> {
        if let Some((net, device)) = self.shaped.device_longest_match_for_ip(ip) {
            return Some((net, device));
        }

        let lookup = match ip.as_ip() {
            IpAddr::V4(ip) => ip.to_ipv6_mapped(),
            IpAddr::V6(ip) => ip,
        };
        let (net, idx) = self.dyn_ip_table.longest_match(lookup)?;
        self.dynamic.get(*idx).map(|circuit| (net, &circuit.shaped))
    }

    /// Returns true if the device hash is currently tracked as a dynamic circuit.
    pub fn is_dynamic_device_hash(&self, device_hash: i64) -> bool {
        self.dyn_by_device_hash.contains_key(&device_hash)
    }

    /// Returns true if the circuit hash is currently tracked as a dynamic circuit.
    pub fn is_dynamic_circuit_hash(&self, circuit_hash: i64) -> bool {
        self.dyn_by_circuit_hash.contains_key(&circuit_hash)
    }

    /// Looks up a shaped device using optional hashes, preferring static shaped devices.
    ///
    /// When the hashes are not present in `ShapedDevices.csv`, this falls back to the
    /// runtime dynamic circuit overlay snapshot.
    pub fn device_by_hashes(
        &self,
        device_hash: Option<i64>,
        circuit_hash: Option<i64>,
    ) -> Option<&ShapedDevice> {
        if let Some(device) = self.shaped.device_by_hashes(device_hash, circuit_hash) {
            return Some(device);
        }

        if let Some(device_hash) = device_hash
            && let Some(idx) = self.dyn_by_device_hash.get(&device_hash)
        {
            return self.dynamic.get(*idx).map(|circuit| &circuit.shaped);
        }

        if let Some(circuit_hash) = circuit_hash
            && let Some(idx) = self.dyn_by_circuit_hash.get(&circuit_hash)
        {
            return self.dynamic.get(*idx).map(|circuit| &circuit.shaped);
        }

        None
    }

    /// Looks up a dynamic circuit overlay entry by circuit id.
    pub fn dynamic_device_by_circuit_id(&self, circuit_id: &str) -> Option<&ShapedDevice> {
        let key = normalize_circuit_id_key(circuit_id);
        let idx = self.dyn_by_circuit_id.get(&key)?;
        self.dynamic.get(*idx).map(|circuit| &circuit.shaped)
    }
}

pub(crate) fn mapped_circuit_count_for_devices<'a>(
    devices: impl IntoIterator<Item = &'a ShapedDevice>,
) -> usize {
    unique_mapped_circuit_hashes(devices.into_iter().filter_map(|device| {
        let has_valid_mapping = device
            .ipv4
            .iter()
            .any(|(_, prefix)| is_valid_ipv4_prefix(*prefix))
            || device
                .ipv6
                .iter()
                .any(|(_, prefix)| is_valid_ipv6_prefix(*prefix));
        has_valid_mapping.then_some(device.circuit_hash)
    }))
    .len()
}

#[cfg(test)]
mod tests {
    use super::{NetworkDevicesCatalog, mapped_circuit_count_for_devices};
    use crate::{DynamicCircuit, ShapedDevicesCatalog};
    use lqos_config::{ConfigShapedDevices, ShapedDevice};
    use std::net::{Ipv4Addr, Ipv6Addr};
    use std::sync::Arc;

    fn device(circuit_hash: i64) -> ShapedDevice {
        ShapedDevice {
            circuit_hash,
            ipv4: vec![(Ipv4Addr::new(192, 0, 2, 1), 32)],
            ..ShapedDevice::default()
        }
    }

    fn catalog(
        static_devices: Vec<ShapedDevice>,
        dynamic_devices: Vec<ShapedDevice>,
    ) -> NetworkDevicesCatalog {
        let shaped = ConfigShapedDevices {
            devices: static_devices,
            ..ConfigShapedDevices::default()
        };
        let dynamic = dynamic_devices
            .into_iter()
            .map(|shaped| DynamicCircuit {
                shaped,
                last_seen_unix: 0,
            })
            .collect();
        NetworkDevicesCatalog::from_snapshots(
            ShapedDevicesCatalog::from_shaped_devices(Arc::new(shaped)),
            Arc::new(dynamic),
        )
    }

    #[test]
    fn mapped_circuit_count_deduplicates_static_and_dynamic_devices() {
        let catalog = catalog(
            vec![device(10), device(10)],
            vec![device(10), device(20), device(20)],
        );

        assert_eq!(
            mapped_circuit_count_for_devices(catalog.iter_all_devices()),
            2
        );
    }

    #[test]
    fn mapped_circuit_count_ignores_unmapped_rows_and_counts_ipv6() {
        let mut unmapped = device(20);
        unmapped.ipv4.clear();
        let mut ipv6 = device(30);
        ipv6.ipv4.clear();
        ipv6.ipv6 = vec![(Ipv6Addr::LOCALHOST, 128)];
        let catalog = catalog(vec![device(10), unmapped], vec![ipv6]);

        assert_eq!(
            mapped_circuit_count_for_devices(catalog.iter_all_devices()),
            2
        );
    }

    #[test]
    fn mapped_circuit_count_ignores_invalid_prefixes() {
        let mut invalid_ipv4 = device(20);
        invalid_ipv4.ipv4 = vec![(Ipv4Addr::new(192, 0, 2, 2), 64)];
        let mut invalid_ipv6 = device(30);
        invalid_ipv6.ipv4.clear();
        invalid_ipv6.ipv6 = vec![(Ipv6Addr::LOCALHOST, 129)];
        let catalog = catalog(vec![device(10), invalid_ipv4], vec![invalid_ipv6]);

        assert_eq!(
            mapped_circuit_count_for_devices(catalog.iter_all_devices()),
            1
        );
    }
}
