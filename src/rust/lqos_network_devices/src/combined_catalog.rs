use crate::{DynamicCircuit, ShapedDevicesCatalog};
use fxhash::FxHashMap;
use lqos_config::ShapedDevice;
use std::sync::Arc;

fn normalize_circuit_id_key(circuit_id: &str) -> String {
    circuit_id.trim().to_ascii_lowercase()
}

/// Snapshot handle for shaped devices plus runtime dynamic circuits.
///
/// This catalog is intended for read-heavy paths (dashboards, APIs) that need to
/// treat dynamic circuits as first-class circuits alongside `ShapedDevices.csv`.
#[derive(Clone)]
pub struct NetworkDevicesCatalog {
    shaped: ShapedDevicesCatalog,
    dynamic: Arc<Vec<DynamicCircuit>>,
    dyn_by_device_hash: FxHashMap<i64, usize>,
    dyn_by_circuit_hash: FxHashMap<i64, usize>,
    dyn_by_circuit_id: FxHashMap<String, usize>,
}

impl NetworkDevicesCatalog {
    /// Builds a combined catalog from explicit snapshots.
    pub fn from_snapshots(
        shaped: ShapedDevicesCatalog,
        dynamic: Arc<Vec<DynamicCircuit>>,
    ) -> Self {
        let mut dyn_by_device_hash = FxHashMap::default();
        let mut dyn_by_circuit_hash = FxHashMap::default();
        let mut dyn_by_circuit_id = FxHashMap::default();

        for (idx, circuit) in dynamic.iter().enumerate() {
            dyn_by_device_hash.insert(circuit.shaped.device_hash, idx);
            dyn_by_circuit_hash.insert(circuit.shaped.circuit_hash, idx);
            dyn_by_circuit_id.insert(normalize_circuit_id_key(&circuit.shaped.circuit_id), idx);
        }

        Self {
            shaped,
            dynamic,
            dyn_by_device_hash,
            dyn_by_circuit_hash,
            dyn_by_circuit_id,
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
        self.iter_static_devices().chain(self.iter_dynamic_devices())
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
