use lqos_config::Config;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

/// Root MQ handle major reserved by LibreQoS.
pub(crate) const MQ_ROOT_HANDLE_MAJOR: u16 = 0x7FFF;
/// Prefer allocating dynamic circuit qdiscs from the upper handle space first.
const PREFERRED_CIRCUIT_START: u16 = 0x9000;
/// Highest qdisc handle major we will allocate dynamically.
const MAX_DYNAMIC_HANDLE_MAJOR: u16 = 0xFFFE;
/// Base range used for deterministic per-queue infrastructure qdisc handles.
const INFRA_HANDLE_BASE: u16 = 0x2000;
/// JSON version for persisted Bakery qdisc-handle state.
const QDISC_HANDLE_FILE_VERSION: u32 = 1;
const QDISC_HANDLE_FILE: &str = "bakery_qdisc_handles.json";

/// Per-device MQ queue layout used to reserve non-circuit qdisc handles.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct MqDeviceLayout {
    devices: BTreeMap<String, BTreeSet<u16>>,
}

impl MqDeviceLayout {
    /// Builds the reserved queue-major layout from the current MQ setup command.
    pub(crate) fn from_setup(
        config: &Config,
        queues_available: usize,
        stick_offset: usize,
    ) -> Self {
        let mut devices = BTreeMap::<String, BTreeSet<u16>>::new();

        let isp = config.isp_interface().to_string();
        let isp_entry = devices.entry(isp).or_default();
        for queue in 0..queues_available {
            let major = (queue + 1) as u16;
            isp_entry.insert(major);
        }

        let internet = config.internet_interface().to_string();
        let internet_entry = devices.entry(internet).or_default();
        for queue in 0..queues_available {
            let major = (queue + stick_offset + 1) as u16;
            internet_entry.insert(major);
        }

        Self { devices }
    }

    /// Returns the qdisc-handle majors reserved on a single interface.
    pub(crate) fn reserved_handles(&self, interface: &str) -> HashSet<u16> {
        let mut reserved = HashSet::new();
        reserved.insert(MQ_ROOT_HANDLE_MAJOR);
        reserved.insert(0xFFFF);

        if let Some(majors) = self.devices.get(interface) {
            for major in majors {
                reserved.insert(*major);
                reserved.insert(infra_qdisc_handle(*major, InfraQdiscSlot::Primary));
                reserved.insert(infra_qdisc_handle(*major, InfraQdiscSlot::Default));
            }
        }

        reserved
    }
}

/// Slot identifier for fixed per-queue infrastructure qdiscs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InfraQdiscSlot {
    Primary,
    Default,
}

/// Computes the deterministic explicit qdisc handle for a fixed infrastructure leaf.
pub(crate) fn infra_qdisc_handle(queue_major: u16, slot: InfraQdiscSlot) -> u16 {
    let base = INFRA_HANDLE_BASE.saturating_add(queue_major.saturating_mul(2));
    match slot {
        InfraQdiscSlot::Primary => base,
        InfraQdiscSlot::Default => base.saturating_add(1),
    }
}

/// Persisted circuit-to-qdisc handle assignments, grouped by interface.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct QdiscHandleState {
    interfaces: BTreeMap<String, InterfaceHandleState>,
    retired_handles: BTreeMap<String, BTreeSet<u16>>,
}

impl QdiscHandleState {
    /// Loads qdisc-handle state from the LibreQoS runtime directory.
    pub(crate) fn load(config: &Config) -> Self {
        let path = state_path(config);
        let Ok(raw) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        let Ok(file) = serde_json::from_str::<PersistedQdiscHandleState>(&raw) else {
            warn!(
                "Bakery: ignoring unreadable qdisc-handle state at {:?}",
                path
            );
            return Self::default();
        };
        if file.version != QDISC_HANDLE_FILE_VERSION {
            debug!(
                "Bakery: ignoring qdisc-handle state version {} at {:?}",
                file.version, path
            );
            return Self::default();
        }

        let interfaces = file
            .interfaces
            .into_iter()
            .map(|(iface, entry)| {
                let circuits = entry
                    .circuits
                    .into_iter()
                    .filter_map(|(hash, handle)| hash.parse::<i64>().ok().map(|h| (h, handle)))
                    .collect();
                (iface, InterfaceHandleState::from_circuits(circuits))
            })
            .collect();

        Self {
            interfaces,
            retired_handles: BTreeMap::new(),
        }
    }

    /// Clears transient retired-handle reservations from the previous apply cycle.
    pub(crate) fn clear_retired_handles(&mut self) {
        for (interface, retired) in self.retired_handles.iter() {
            let Some(state) = self.interfaces.get_mut(interface) else {
                continue;
            };
            for handle in retired {
                state.note_reusable_handle(*handle);
            }
        }
        self.retired_handles.clear();
    }

    /// Saves qdisc-handle state to the LibreQoS runtime directory.
    pub(crate) fn save(&self, config: &Config) {
        let path = state_path(config);
        if let Some(parent) = path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            warn!(
                "Bakery: unable to create qdisc-handle state directory {:?}: {}",
                parent, e
            );
            return;
        }

        let persisted = PersistedQdiscHandleState {
            version: QDISC_HANDLE_FILE_VERSION,
            interfaces: self
                .interfaces
                .iter()
                .map(|(iface, circuits)| {
                    (
                        iface.clone(),
                        PersistedInterfaceHandleState {
                            circuits: circuits
                                .circuits
                                .iter()
                                .map(|(hash, handle)| (hash.to_string(), *handle))
                                .collect(),
                        },
                    )
                })
                .collect(),
        };

        let Ok(serialized) = serde_json::to_string_pretty(&persisted) else {
            warn!("Bakery: unable to serialize qdisc-handle state");
            return;
        };

        let temp_path = path.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&temp_path, serialized.as_bytes()) {
            warn!(
                "Bakery: unable to write temporary qdisc-handle state {:?}: {}",
                temp_path, e
            );
            return;
        }

        if let Err(e) = std::fs::rename(&temp_path, &path) {
            warn!(
                "Bakery: unable to atomically replace qdisc-handle state {:?}: {}",
                path, e
            );
            let _ = std::fs::remove_file(&temp_path);
        }
    }

    /// Returns a stable explicit qdisc handle for the circuit on the given interface.
    pub(crate) fn assign_circuit_handle(
        &mut self,
        interface: &str,
        circuit_hash: i64,
        reserved: &HashSet<u16>,
    ) -> Option<u16> {
        if let Some(existing) = self
            .interfaces
            .get(interface)
            .and_then(|state| state.circuits.get(&circuit_hash))
            .copied()
            && !reserved.contains(&existing)
            && self
                .interfaces
                .get(interface)
                .and_then(|state| state.owner_of(existing))
                == Some(circuit_hash)
        {
            return Some(existing);
        }

        let retired = self.retired_handles.get(interface);
        let state = self.interfaces.entry(interface.to_string()).or_default();
        let next = state.next_free_handle(reserved, retired)?;
        state.insert(circuit_hash, next);
        Some(next)
    }

    /// Forces a circuit to rotate to a different explicit qdisc handle on an interface.
    pub(crate) fn rotate_circuit_handle(
        &mut self,
        interface: &str,
        circuit_hash: i64,
        reserved: &HashSet<u16>,
    ) -> Option<u16> {
        let retired_handle = self
            .interfaces
            .get(interface)
            .and_then(|state| state.circuits.get(&circuit_hash))
            .copied();

        let retired = self.retired_handles.get(interface);
        let state = self.interfaces.entry(interface.to_string()).or_default();
        let next = state.next_free_handle(reserved, retired)?;
        if let Some(handle) = retired_handle {
            self.retired_handles
                .entry(interface.to_string())
                .or_default()
                .insert(handle);
        }
        state.insert(circuit_hash, next);
        Some(next)
    }

    /// Releases a circuit handle on an interface so it can be reused deterministically.
    pub(crate) fn release_circuit(&mut self, interface: &str, circuit_hash: i64) -> bool {
        self.interfaces
            .get_mut(interface)
            .and_then(|state| state.remove(circuit_hash))
            .is_some()
    }

    /// Drops persisted handle assignments for circuits no longer present on an interface.
    pub(crate) fn retain_circuits(&mut self, interface: &str, active: &HashSet<i64>) {
        if let Some(state) = self.interfaces.get_mut(interface) {
            state.retain_circuits(active);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InterfaceHandleState {
    circuits: BTreeMap<i64, u16>,
    owners: HashMap<u16, i64>,
    next_candidate: u16,
}

impl Default for InterfaceHandleState {
    fn default() -> Self {
        Self {
            circuits: BTreeMap::new(),
            owners: HashMap::new(),
            next_candidate: PREFERRED_CIRCUIT_START,
        }
    }
}

impl InterfaceHandleState {
    fn from_circuits(circuits: BTreeMap<i64, u16>) -> Self {
        let mut state = Self {
            circuits,
            owners: HashMap::new(),
            next_candidate: PREFERRED_CIRCUIT_START,
        };
        for (circuit_hash, handle) in state.circuits.iter() {
            state.owners.insert(*handle, *circuit_hash);
        }
        state
    }

    fn owner_of(&self, handle: u16) -> Option<i64> {
        self.owners.get(&handle).copied()
    }

    fn insert(&mut self, circuit_hash: i64, handle: u16) {
        if let Some(previous) = self.circuits.insert(circuit_hash, handle) {
            self.owners.remove(&previous);
            self.note_reusable_handle(previous);
        }
        self.owners.insert(handle, circuit_hash);
        if self.next_candidate == handle {
            self.next_candidate = advance_handle_candidate(handle);
        }
    }

    fn remove(&mut self, circuit_hash: i64) -> Option<u16> {
        let removed = self.circuits.remove(&circuit_hash)?;
        self.owners.remove(&removed);
        self.note_reusable_handle(removed);
        Some(removed)
    }

    fn retain_circuits(&mut self, active: &HashSet<i64>) {
        let removed = self
            .circuits
            .iter()
            .filter_map(|(hash, handle)| (!active.contains(hash)).then_some((*hash, *handle)))
            .collect::<Vec<_>>();

        for (hash, handle) in removed {
            self.circuits.remove(&hash);
            self.owners.remove(&handle);
            self.note_reusable_handle(handle);
        }
    }

    fn note_reusable_handle(&mut self, handle: u16) {
        if handle >= PREFERRED_CIRCUIT_START || self.next_candidate < PREFERRED_CIRCUIT_START {
            self.next_candidate = self.next_candidate.min(handle);
        } else if self.circuits.is_empty() {
            self.next_candidate = PREFERRED_CIRCUIT_START;
        }
    }

    fn next_free_handle(
        &mut self,
        reserved: &HashSet<u16>,
        retired: Option<&BTreeSet<u16>>,
    ) -> Option<u16> {
        if let Some(handle) = self.scan_available(
            self.next_candidate,
            MAX_DYNAMIC_HANDLE_MAJOR,
            reserved,
            retired,
        ) {
            self.next_candidate = advance_handle_candidate(handle);
            return Some(handle);
        }

        if self.next_candidate > PREFERRED_CIRCUIT_START
            && let Some(handle) = self.scan_available(
                PREFERRED_CIRCUIT_START,
                self.next_candidate - 1,
                reserved,
                retired,
            )
        {
            self.next_candidate = advance_handle_candidate(handle);
            return Some(handle);
        }

        let low_end = PREFERRED_CIRCUIT_START.saturating_sub(1);
        let low_start = if self.next_candidate < PREFERRED_CIRCUIT_START {
            self.next_candidate.max(1)
        } else {
            1
        };

        if let Some(handle) = self.scan_available(low_start, low_end, reserved, retired) {
            self.next_candidate = advance_handle_candidate(handle);
            return Some(handle);
        }

        if low_start > 1
            && let Some(handle) = self.scan_available(1, low_start - 1, reserved, retired)
        {
            self.next_candidate = advance_handle_candidate(handle);
            return Some(handle);
        }

        None
    }

    fn scan_available(
        &self,
        start: u16,
        end: u16,
        reserved: &HashSet<u16>,
        retired: Option<&BTreeSet<u16>>,
    ) -> Option<u16> {
        if start > end {
            return None;
        }

        for handle in start..=end {
            if reserved.contains(&handle) {
                continue;
            }
            if self.owners.contains_key(&handle) {
                continue;
            }
            if retired.is_some_and(|set| set.contains(&handle)) {
                continue;
            }
            return Some(handle);
        }
        None
    }
}

fn advance_handle_candidate(handle: u16) -> u16 {
    if handle >= MAX_DYNAMIC_HANDLE_MAJOR {
        1
    } else {
        handle + 1
    }
}

fn state_path(config: &Config) -> PathBuf {
    Path::new(&config.lqos_directory).join(QDISC_HANDLE_FILE)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedQdiscHandleState {
    version: u32,
    #[serde(default)]
    interfaces: BTreeMap<String, PersistedInterfaceHandleState>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PersistedInterfaceHandleState {
    #[serde(default)]
    circuits: BTreeMap<String, u16>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_primary_default_handles_are_unique() {
        let primary = infra_qdisc_handle(1, InfraQdiscSlot::Primary);
        let default = infra_qdisc_handle(1, InfraQdiscSlot::Default);
        assert_ne!(primary, default);
        assert_eq!(primary, 0x2002);
        assert_eq!(default, 0x2003);
    }

    #[test]
    fn allocator_prefers_high_range_and_reuses_existing() {
        let mut state = QdiscHandleState::default();
        let reserved = HashSet::from([0x7FFF, 0xFFFF]);
        let a = state
            .assign_circuit_handle("eth0", 1, &reserved)
            .expect("first handle");
        let b = state
            .assign_circuit_handle("eth0", 2, &reserved)
            .expect("second handle");
        assert_eq!(a, PREFERRED_CIRCUIT_START);
        assert_eq!(b, PREFERRED_CIRCUIT_START + 1);
        let a_again = state
            .assign_circuit_handle("eth0", 1, &reserved)
            .expect("reused handle");
        assert_eq!(a, a_again);
    }

    #[test]
    fn allocator_can_rotate_to_a_new_handle() {
        let mut state = QdiscHandleState::default();
        let reserved = HashSet::from([0x7FFF, 0xFFFF]);
        let first = state
            .assign_circuit_handle("eth0", 1, &reserved)
            .expect("first handle");
        let rotated = state
            .rotate_circuit_handle("eth0", 1, &reserved)
            .expect("rotated handle");
        assert_ne!(first, rotated);
        assert_eq!(rotated, first + 1);
    }

    #[test]
    fn rotated_handles_are_not_reused_until_cleared() {
        let mut state = QdiscHandleState::default();
        let reserved = HashSet::from([0x7FFF, 0xFFFF]);
        let first = state
            .assign_circuit_handle("eth0", 1, &reserved)
            .expect("first handle");
        let rotated = state
            .rotate_circuit_handle("eth0", 1, &reserved)
            .expect("rotated handle");
        let second_circuit = state
            .assign_circuit_handle("eth0", 2, &reserved)
            .expect("second circuit handle");
        assert_ne!(second_circuit, first);
        assert_eq!(second_circuit, rotated + 1);

        state.clear_retired_handles();
        let third_circuit = state
            .assign_circuit_handle("eth0", 3, &reserved)
            .expect("third circuit handle");
        assert_eq!(third_circuit, first);
    }
}
