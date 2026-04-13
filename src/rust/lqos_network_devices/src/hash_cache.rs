use fxhash::FxHashMap;
use lqos_config::ShapedDevice;

#[derive(Debug, Default)]
pub struct ShapedDeviceHashCache {
    by_device_hash: FxHashMap<i64, usize>,
    by_circuit_hash: FxHashMap<i64, usize>,
}

impl ShapedDeviceHashCache {
    pub fn from_devices(devices: &[ShapedDevice]) -> Self {
        let mut by_device_hash = FxHashMap::default();
        by_device_hash.reserve(devices.len());
        let mut by_circuit_hash = FxHashMap::default();
        by_circuit_hash.reserve(devices.len());
        for (idx, dev) in devices.iter().enumerate() {
            by_device_hash.insert(dev.device_hash, idx);
            by_circuit_hash.entry(dev.circuit_hash).or_insert(idx);
        }
        Self {
            by_device_hash,
            by_circuit_hash,
        }
    }

    pub fn index_by_device_hash(
        &self,
        shaped: &lqos_config::ConfigShapedDevices,
        device_hash: i64,
    ) -> Option<usize> {
        if let Some(idx) = self.by_device_hash.get(&device_hash).copied()
            && shaped
                .devices
                .get(idx)
                .is_some_and(|d| d.device_hash == device_hash)
        {
            return Some(idx);
        }
        shaped
            .devices
            .iter()
            .position(|d| d.device_hash == device_hash)
    }

    pub fn index_by_circuit_hash(
        &self,
        shaped: &lqos_config::ConfigShapedDevices,
        circuit_hash: i64,
    ) -> Option<usize> {
        if let Some(idx) = self.by_circuit_hash.get(&circuit_hash).copied()
            && shaped
                .devices
                .get(idx)
                .is_some_and(|d| d.circuit_hash == circuit_hash)
        {
            return Some(idx);
        }
        shaped
            .devices
            .iter()
            .position(|d| d.circuit_hash == circuit_hash)
    }
}
