use crate::hash_cache::ShapedDeviceHashCache;
use fxhash::FxHashSet;
use ip_network::IpNetwork;
use lqos_config::{ConfigShapedDevices, ShapedDevice};
use lqos_utils::XdpIpAddress;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;

/// Snapshot handle for shaped devices plus derived lookup structures.
///
/// Prefer using this catalog over reaching into `ConfigShapedDevices` directly.
/// It keeps related snapshots consistent (shaped devices + hash cache) and exposes
/// common operations as verb-style methods.
#[derive(Clone)]
pub struct ShapedDevicesCatalog {
    shaped: Arc<ConfigShapedDevices>,
    cache: Arc<ShapedDeviceHashCache>,
    generation: u64,
}

impl ShapedDevicesCatalog {
    pub(crate) fn new(
        shaped: Arc<ConfigShapedDevices>,
        cache: Arc<ShapedDeviceHashCache>,
        generation: u64,
    ) -> Self {
        Self {
            shaped,
            cache,
            generation,
        }
    }

    /// Builds a catalog from an arbitrary shaped-devices snapshot.
    ///
    /// This is intended for one-shot tools that load `ShapedDevices.csv` from disk
    /// and want the higher-level lookup helpers without starting the runtime actor.
    pub fn from_shaped_devices(shaped: Arc<ConfigShapedDevices>) -> Self {
        let cache = Arc::new(ShapedDeviceHashCache::from_devices(&shaped.devices));
        Self {
            shaped,
            cache,
            generation: 0,
        }
    }

    /// Returns the monotonic generation number of the underlying shaped-devices snapshot.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Returns the number of shaped-device rows in the snapshot.
    pub fn devices_len(&self) -> usize {
        self.shaped.devices.len()
    }

    /// Iterates over all shaped-device rows in the snapshot.
    pub fn iter_devices(&self) -> impl Iterator<Item = &ShapedDevice> {
        self.shaped.devices.iter()
    }

    /// Returns an owned copy of all shaped-device rows.
    pub fn clone_all_devices(&self) -> Vec<ShapedDevice> {
        self.shaped.devices.clone()
    }

    /// Looks up a shaped device by the stable `device_hash` field.
    pub fn device_by_device_hash(&self, device_hash: i64) -> Option<&ShapedDevice> {
        self.cache
            .index_by_device_hash(&self.shaped, device_hash)
            .and_then(|idx| self.shaped.devices.get(idx))
    }

    /// Looks up a shaped device by the stable `circuit_hash` field.
    pub fn device_by_circuit_hash(&self, circuit_hash: i64) -> Option<&ShapedDevice> {
        self.cache
            .index_by_circuit_hash(&self.shaped, circuit_hash)
            .and_then(|idx| self.shaped.devices.get(idx))
    }

    /// Looks up a shaped device using optional hashes, preferring `device_hash`
    /// over `circuit_hash` when both are present.
    pub fn device_by_hashes(
        &self,
        device_hash: Option<i64>,
        circuit_hash: Option<i64>,
    ) -> Option<&ShapedDevice> {
        if let Some(device_hash) = device_hash
            && let Some(device) = self.device_by_device_hash(device_hash)
        {
            return Some(device);
        }
        if let Some(circuit_hash) = circuit_hash
            && let Some(device) = self.device_by_circuit_hash(circuit_hash)
        {
            return Some(device);
        }
        None
    }

    /// Returns a cloned list of shaped-device rows that belong to a circuit identifier.
    pub fn devices_for_circuit_id(&self, circuit_id: &str) -> Vec<ShapedDevice> {
        let safe_id = circuit_id.to_lowercase().trim().to_string();
        self.shaped
            .devices
            .iter()
            .filter(|device| device.circuit_id.to_lowercase().trim() == safe_id)
            .cloned()
            .collect()
    }

    /// Returns the count of unique configured circuits present in the shaped-devices snapshot.
    pub fn configured_circuit_count(&self) -> usize {
        let mut circuits: FxHashSet<&str> = FxHashSet::default();
        circuits.reserve(self.shaped.devices.len());
        for device in &self.shaped.devices {
            let circuit_id = device.circuit_id.trim();
            if circuit_id.is_empty() {
                continue;
            }
            circuits.insert(circuit_id);
        }
        circuits.len()
    }

    /// Returns a per-circuit map of maximum configured rates, using the first-seen parent node.
    pub fn circuit_rate_caps_by_circuit_id(&self) -> HashMap<String, CircuitRateCaps> {
        let mut by_circuit_id: HashMap<String, CircuitRateCaps> = HashMap::new();
        for device in &self.shaped.devices {
            let entry = by_circuit_id
                .entry(device.circuit_id.clone())
                .or_insert_with(|| CircuitRateCaps {
                    parent_node: device.parent_node.clone(),
                    download_max_mbps: 0.0,
                    upload_max_mbps: 0.0,
                });
            entry.download_max_mbps = entry.download_max_mbps.max(device.download_max_mbps);
            entry.upload_max_mbps = entry.upload_max_mbps.max(device.upload_max_mbps);
        }
        by_circuit_id
    }

    /// Looks up the `circuit_hash` for an IP address using the shaped-devices LPM trie.
    pub fn circuit_hash_for_ip(&self, ip: &XdpIpAddress) -> Option<i64> {
        self.shaped.get_circuit_hash_from_ip(ip)
    }

    /// Looks up a `(circuit_id, circuit_name)` pair for an IP address using the shaped-devices LPM trie.
    pub fn circuit_id_and_name_for_ip(&self, ip: &XdpIpAddress) -> Option<(String, String)> {
        self.shaped.get_circuit_id_and_name_from_ip(ip)
    }

    /// Returns the longest-prefix match entry for an IP address using the shaped-devices LPM trie.
    pub fn device_longest_match_for_ip(
        &self,
        ip: &XdpIpAddress,
    ) -> Option<(IpNetwork, &ShapedDevice)> {
        let lookup = match ip.as_ip() {
            IpAddr::V4(ip) => ip.to_ipv6_mapped(),
            IpAddr::V6(ip) => ip,
        };
        let (net, idx) = self.shaped.trie.longest_match(lookup)?;
        self.shaped.devices.get(*idx).map(|dev| (net, dev))
    }

    /// Iterates over all IP network mappings in the shaped-devices LPM trie.
    pub fn iter_ip_mappings(&self) -> impl Iterator<Item = (IpNetwork, &ShapedDevice)> {
        self.shaped
            .trie
            .iter()
            .filter_map(|(net, &idx)| self.shaped.devices.get(idx).map(|dev| (net, dev)))
    }
}

/// Canonical per-circuit configured rate caps derived from shaped-device rows.
#[derive(Clone, Debug, PartialEq)]
pub struct CircuitRateCaps {
    pub parent_node: String,
    pub download_max_mbps: f32,
    pub upload_max_mbps: f32,
}
