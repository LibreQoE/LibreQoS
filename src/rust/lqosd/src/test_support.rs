use crate::throughput_tracker::flow_data::{
    FlowAnalysis, FlowbeeLocalData, active_flow_test_lock, replace_active_flows_for_test,
};
use lqos_config::{ConfigShapedDevices, ShapedDevice};
use lqos_sys::flowbee_data::{FlowbeeData, FlowbeeKey};
use lqos_utils::XdpIpAddress;
use lqos_utils::units::DownUpOrder;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

/// Serializes tests that mutate process-global runtime configuration state.
pub(crate) fn runtime_config_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Holds global active-flow snapshot state for tests and restores replaced shaped-device state on drop.
pub(crate) struct ActiveFlowSnapshotTestContext {
    _runtime_guard: Option<MutexGuard<'static, ()>>,
    _active_flow_guard: MutexGuard<'static, ()>,
    old_shaped: Option<Arc<ConfigShapedDevices>>,
}

impl ActiveFlowSnapshotTestContext {
    /// Creates an active-flow test context without changing shaped-device catalog state.
    pub(crate) fn empty() -> Self {
        let active_flow_guard = active_flow_test_lock();
        replace_active_flows_for_test(Vec::new());
        Self {
            _runtime_guard: None,
            _active_flow_guard: active_flow_guard,
            old_shaped: None,
        }
    }

    /// Creates an active-flow test context with a temporary shaped-device catalog.
    pub(crate) fn with_shaped_devices(reason: &str, devices: Vec<ShapedDevice>) -> Self {
        let runtime_guard = runtime_config_test_lock()
            .lock()
            .expect("runtime config test lock should not be poisoned");
        let active_flow_guard = active_flow_test_lock();
        let mut shaped = ConfigShapedDevices::default();
        shaped.replace_with_new_data(devices);
        let old_shaped = lqos_network_devices::swap_shaped_devices_snapshot(reason, Arc::new(shaped));
        replace_active_flows_for_test(Vec::new());

        Self {
            _runtime_guard: Some(runtime_guard),
            _active_flow_guard: active_flow_guard,
            old_shaped: Some(old_shaped),
        }
    }

    /// Replaces the shaped-device catalog while preserving the original catalog for restore.
    pub(crate) fn replace_shaped_devices(&mut self, reason: &str, devices: Vec<ShapedDevice>) {
        let mut shaped = ConfigShapedDevices::default();
        shaped.replace_with_new_data(devices);
        let replacement = Arc::new(shaped);
        if let Some(original) = self.old_shaped.take() {
            lqos_network_devices::swap_shaped_devices_snapshot(reason, replacement);
            self.old_shaped = Some(original);
        } else {
            self.old_shaped = Some(lqos_network_devices::swap_shaped_devices_snapshot(
                reason,
                replacement,
            ));
        }
    }
}

impl Drop for ActiveFlowSnapshotTestContext {
    fn drop(&mut self) {
        replace_active_flows_for_test(Vec::new());
        if let Some(old_shaped) = self.old_shaped.take() {
            lqos_network_devices::swap_shaped_devices_snapshot(
                "active-flow-snapshot-test-restore",
                old_shaped,
            );
        }
    }
}

/// Builds a live active-flow map entry for tests that seed the published snapshot.
pub(crate) fn active_flow_entry(
    local_ip: [u8; 4],
    remote_ip: [u8; 4],
    dst_port: u16,
    last_seen: u64,
    rate: DownUpOrder<u32>,
    bytes: DownUpOrder<u64>,
    packets: DownUpOrder<u64>,
) -> (FlowbeeKey, (FlowbeeLocalData, FlowAnalysis)) {
    let mut key = FlowbeeKey::default();
    key.local_ip = XdpIpAddress::from_ip(IpAddr::from(local_ip));
    key.remote_ip = XdpIpAddress::from_ip(IpAddr::from(remote_ip));
    key.ip_protocol = 6;
    key.src_port = 443;
    key.dst_port = dst_port;

    let mut raw = FlowbeeData::default();
    raw.start_time = last_seen.saturating_sub(1_000_000_000);
    raw.last_seen = last_seen;
    raw.bytes_sent = bytes;
    raw.packets_sent = packets;
    raw.rate_estimate_bps = rate;
    raw.tcp_retransmits = DownUpOrder::new(1, 2);
    raw.flags = 0x12;

    (
        key,
        (
            FlowbeeLocalData::from_flow(&raw, &key),
            FlowAnalysis::new(&key),
        ),
    )
}
