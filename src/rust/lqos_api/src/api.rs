//! Provides web API endpoints for LibreQoS nodes.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

mod active_users;
mod asn_top;
mod circuit_rollups;
mod circuit_stats;
mod circuits_at_capacity;
mod circuits_high_retransmits;
mod combined_status;
mod counts;
mod cpu_workers;
mod current_throughput;
mod flow_explorer;
mod flows;
mod health_check;
mod host_counters;
mod host_counts;
mod lqosd_control;
mod missing_users;
mod network_map;
mod node_manager_extras;
mod ping_circuit;
mod ping_ip;
mod protocols;
mod qoo;
mod rtt_histogram;
mod shaped_devices;
mod shaped_unshaped;
mod status;
mod stormguard_status;
mod system_metrics;
mod top_n;
mod traceroute_circuit;
mod traceroute_ip;
mod tree_nodes_at_capacity;
mod tree_nodes_high_retransmits;
mod tree_summary;
mod unknown_ips; // Re-enabled: endpoint and bus support available
pub(crate) mod version;
// ticket_suggestion removed with chatbot
mod configuration;
mod network_json;
mod overrides;
mod scheduler_status;
mod warnings;

pub use active_users::*;
pub use asn_top::*;
pub use circuit_rollups::*;
pub use circuit_stats::*;
pub use circuits_at_capacity::*;
pub use circuits_high_retransmits::*;
pub use combined_status::*;
pub use configuration::*;
pub use counts::*;
pub use cpu_workers::*;
pub use current_throughput::*;
pub use flow_explorer::*;
pub use flows::*;
pub use health_check::*;
pub use host_counters::*;
pub use host_counts::*;
pub use lqosd_control::*;
pub use missing_users::*;
pub use network_json::*;
pub use network_map::*;
pub use node_manager_extras::*;
pub use overrides::*;
pub use ping_circuit::*;
pub use ping_ip::*;
pub use protocols::*;
pub use qoo::*;
pub use rtt_histogram::*;
pub use scheduler_status::*;
pub use shaped_devices::*;
pub use shaped_unshaped::*;
pub use status::*;
pub use stormguard_status::*;
pub use system_metrics::*;
pub use top_n::*;
pub use traceroute_circuit::*;
pub use traceroute_ip::*;
pub use tree_nodes_at_capacity::*;
pub use tree_nodes_high_retransmits::*;
pub use tree_summary::*;
pub use unknown_ips::*;
pub(crate) use version::test_hooks_enabled;
pub use version::*;
pub use warnings::*;

/// The daemon-owned inputs `/api/v3` reads instead of running its own copies.
///
/// Every field is a handle to a source `lqosd` already runs and already shows
/// on its own pages: the mount must not start a second sampler, watcher, or
/// topology detector, because two copies of the same measurement can disagree.
/// The handles are plain values on purpose — no trait, no channel, no spawn.
pub struct ApiState {
    /// The daemon's boot-time shaping-CPU snapshot (the cell its CPU & Workers
    /// page reads). `None` inside the cell = the detector has not run yet.
    cpu_topology: &'static OnceLock<Option<CpuTopologySection>>,
    /// The daemon's 1 Hz CPU/RAM sampler.
    system_stats: fn() -> SystemUsageSnapshot,
    /// The daemon's circuit directory: `circuit_id -> circuit_name`.
    circuit_names: fn() -> HashMap<String, String>,
}

impl ApiState {
    /// Builds the mount state from the daemon's handles.
    pub fn new(
        cpu_topology: &'static OnceLock<Option<CpuTopologySection>>,
        system_stats: fn() -> SystemUsageSnapshot,
        circuit_names: fn() -> HashMap<String, String>,
    ) -> Arc<Self> {
        Arc::new(Self {
            cpu_topology,
            system_stats,
            circuit_names,
        })
    }

    /// The daemon's shaping-CPU snapshot, or the honest "not detected yet" one.
    pub fn cpu_topology(&self) -> CpuTopologySection {
        self.cpu_topology
            .get()
            .and_then(|section| section.clone())
            .unwrap_or_else(|| CpuTopologySection::unavailable("topology_pending"))
    }

    /// The daemon's current CPU/RAM sample.
    pub fn system_stats(&self) -> SystemUsageSnapshot {
        (self.system_stats)()
    }

    /// The daemon's `circuit_id -> circuit_name` directory.
    pub fn circuit_names(&self) -> HashMap<String, String> {
        (self.circuit_names)()
    }
}
