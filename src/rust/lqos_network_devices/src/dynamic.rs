use lqos_config::ShapedDevice;
use lqos_utils::XdpIpAddress;

/// Runtime-only dynamic circuit overlay entry.
///
/// This is an overlay on top of `ShapedDevices.csv` and never mutates the static file.
/// It is intended for circuits/devices that are created and managed dynamically at runtime.
#[derive(Clone, Debug, PartialEq)]
pub struct DynamicCircuit {
    /// Shaped-device-like fields describing the dynamic circuit/device.
    pub shaped: ShapedDevice,
    /// Unix timestamp (seconds) when this dynamic circuit was last observed active.
    pub last_seen_unix: u64,
}

/// Minimal kernel observation used to drive dynamic-circuit activity updates.
///
/// Hash fields come from the kernel IP map; when present they should be preferred over
/// any trie-derived lookups.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CircuitObservation {
    pub ip: XdpIpAddress,
    pub device_hash: Option<i64>,
    pub circuit_hash: Option<i64>,
}
