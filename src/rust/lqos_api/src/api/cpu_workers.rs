//! `lqos-74t.118.2`: read-only CPU & Workers API.
//!
//! The endpoint is assembled from three honest sources:
//! * CPU usage — the daemon's 1 Hz sampler (`ApiState::system_stats`);
//! * CPU topology — the daemon's boot-time shaping-CPU snapshot;
//! * bridge data — lqosd's read-only `BusRequest::GetCpuWorkers` snapshot
//!   (worker affinity, applied ownership/crossings, placement state, moves).
//!
//! Every absent source is represented as `available:false` + `reason` + `null`
//! payload. `null` is never rewritten to `0`/`[]`; counters/throughput/drops
//! are explicitly unsupported in v1. There is no kernel `cpu_affinity` field.

use axum::{Json, extract::State, http::StatusCode};
use lqos_bus::{BusRequest, BusResponse, LibreqosBusClient};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

use crate::api::{ApiState, SystemUsageSnapshot};

#[derive(Serialize, Deserialize, Clone, Debug, ToSchema)]
pub struct CpuWorkersResponse {
    /// `ok` only when every documented source is available (CPU usage,
    /// topology, bridge, workers, ownership, moves); `partial` when at least
    /// one source is available but another is unavailable (v1 moves is always
    /// unsupported, so a working bridge is `partial`); `unavailable` when no
    /// source is available.
    pub state: String,
    /// Honest reasons for every unavailable section (deduped, display order).
    pub unavailable_reasons: Vec<String>,
    /// Unix seconds when this response was assembled.
    pub generated_at_unix: u64,
    /// Per-CPU usage (this process's 1 Hz sampler).
    pub cpu_usage: CpuUsageSection,
    /// Shaping-CPU topology (the boot-time detector; not a worker-lcore claim).
    pub cpu_topology: CpuTopologySection,
    /// Bridge placement/convergence state + configured autobalance.
    pub bridge: BridgeSection,
    /// Bridge worker affinity (`StateReport.workers`).
    pub workers: WorkersSection,
    /// Applied circuit/node ownership sets and crossings.
    pub ownership: OwnershipSection,
    /// Recent ownership moves. v1 has no history source: honest-empty.
    pub moves: MovesSection,
}

#[derive(Serialize, Deserialize, Clone, Debug, ToSchema)]
pub struct CpuUsageSection {
    pub available: bool,
    pub reason: Option<String>,
    pub num_cpus: Option<usize>,
    /// One row per sampled CPU. `cpu_id:null` = the CPU-id join is unproven.
    pub per_cpu: Option<Vec<CpuUsageRow>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, ToSchema)]
pub struct CpuUsageRow {
    pub cpu_id: Option<u32>,
    pub percent: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, ToSchema)]
pub struct CpuTopologySection {
    pub available: bool,
    pub reason: Option<String>,
    pub exclude_efficiency_cores: Option<bool>,
    /// The detector's stable serde spelling (`core_type_sysfs`, ...).
    pub source: Option<String>,
    pub from_cache: Option<bool>,
    pub has_hybrid_split: Option<bool>,
    pub possible: Option<Vec<u32>>,
    pub performance: Option<Vec<u32>>,
    pub efficiency: Option<Vec<u32>>,
    /// The CPU list the shaping path uses (separate from `possible`).
    pub shaping: Option<Vec<u32>>,
    pub detail: Option<String>,
}

/// Stable snake_case spellings for the shaping-CPU detector. Do NOT rely on
/// the enum's serde spelling: that spelling is persisted in the topology cache,
/// so the API owns this explicit mapping.
fn topology_source_str(source: lqos_config::ShapingCpuSource) -> &'static str {
    match source {
        lqos_config::ShapingCpuSource::ExclusionDisabled => "exclusion_disabled",
        lqos_config::ShapingCpuSource::CoreTypeSysfs => "core_type_sysfs",
        lqos_config::ShapingCpuSource::CpuCapacitySysfs => "cpu_capacity_sysfs",
        lqos_config::ShapingCpuSource::CpuCoreSysfs => "cpu_core_sysfs",
        lqos_config::ShapingCpuSource::PossibleMinusAtomSysfs => "possible_minus_atom_sysfs",
        lqos_config::ShapingCpuSource::CpuidHybrid => "cpuid_hybrid",
        lqos_config::ShapingCpuSource::MaxFrequencyHeuristic => "max_frequency_heuristic",
        lqos_config::ShapingCpuSource::FallbackAllPossible => "fallback_all_possible",
        lqos_config::ShapingCpuSource::FallbackAvailableParallelism => {
            "fallback_available_parallelism"
        }
    }
}

impl CpuTopologySection {
    /// The honest "no topology" section, naming why it is absent.
    pub fn unavailable(reason: &str) -> Self {
        Self {
            available: false,
            reason: Some(reason.to_string()),
            exclude_efficiency_cores: None,
            source: None,
            from_cache: None,
            has_hybrid_split: None,
            possible: None,
            performance: None,
            efficiency: None,
            shaping: None,
            detail: None,
        }
    }

    /// The section for a shaping-CPU detection result.
    pub fn from_detection(detection: lqos_config::ShapingCpuDetection) -> Self {
        Self {
            available: true,
            reason: None,
            exclude_efficiency_cores: Some(detection.exclude_efficiency_cores),
            source: Some(topology_source_str(detection.source).to_string()),
            from_cache: Some(detection.from_cache),
            has_hybrid_split: Some(detection.has_hybrid_split),
            possible: Some(detection.possible),
            performance: Some(detection.performance),
            efficiency: Some(detection.efficiency),
            shaping: Some(detection.shaping),
            detail: Some(detection.detail),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, ToSchema)]
pub struct BridgeSection {
    pub available: bool,
    pub reason: Option<String>,
    /// `healthy` | `converging` | `held` (None before the first report).
    pub state: Option<String>,
    pub applied_generation: Option<String>,
    pub desired_generation: Option<String>,
    pub applied_provenance: Option<String>,
    pub applied_diverged: Option<bool>,
    pub pending: Option<String>,
    pub last_reject: Option<String>,
    /// The broker's held maintenance-window record; `null` when nothing is held.
    pub deferred_maintenance: Option<DeferredMaintenanceSection>,
    pub autobalance: AutobalanceSection,
}

impl BridgeSection {
    pub fn unavailable(reason: &str) -> Self {
        Self {
            available: false,
            reason: Some(reason.to_string()),
            state: None,
            applied_generation: None,
            desired_generation: None,
            applied_provenance: None,
            applied_diverged: None,
            pending: None,
            last_reject: None,
            deferred_maintenance: None,
            autobalance: AutobalanceSection::unavailable(reason),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, ToSchema)]
pub struct DeferredMaintenanceSection {
    /// The human label of the held pause class.
    pub class: String,
    /// Unix seconds the held change was FIRST deferred (the force clock).
    pub first_deferred_unix: u64,
    /// The op count of the currently-held coalesced net delta.
    pub held_ops: u64,
    /// Whether the `force_apply_hours` bound has fired for this hold.
    pub forced: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, ToSchema)]
pub struct AutobalanceSection {
    /// The CONFIGURED posture from `bridge.env` (`None` = absent/unreadable).
    pub configured: Option<bool>,
    /// Runtime ACTIVE/INACTIVE is log-only in v1; always `null`.
    pub runtime: Option<bool>,
    pub reason: Option<String>,
}

impl AutobalanceSection {
    fn unavailable(reason: &str) -> Self {
        Self {
            configured: None,
            runtime: None,
            reason: Some(reason.to_string()),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, ToSchema)]
pub struct WorkersSection {
    pub available: bool,
    pub reason: Option<String>,
    pub schema: Option<u32>,
    pub rx_queue_count: Option<u16>,
    pub tx_queue_count: Option<u16>,
    /// `unsupported` in v1; never inferred from zeros.
    pub counter_support: Option<String>,
    pub rows: Option<Vec<WorkerRow>>,
}

impl WorkersSection {
    fn unavailable(reason: &str) -> Self {
        Self {
            available: false,
            reason: Some(reason.to_string()),
            schema: None,
            rx_queue_count: None,
            tx_queue_count: None,
            counter_support: None,
            rows: None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, ToSchema)]
pub struct WorkerRow {
    pub worker_id: u16,
    pub lcore_id: Option<u32>,
    pub numa_node: Option<i32>,
    /// DPDK per-port queue INDICES (valid on both logical ports).
    pub rx_queue_ids: Option<Vec<u16>>,
    pub tx_queue_id: Option<u16>,
    pub shared_tx_queue: Option<bool>,
    /// Circuits currently owned by this worker; `null` when the owner map is
    /// absent (never a fabricated 0).
    pub circuit_count: Option<u32>,
    /// Nodes this worker SOLELY owns (a shared/crossing node is not owned);
    /// `null` when the owner map is absent.
    pub solely_owned_node_count: Option<u32>,
    /// Cumulative counters; v1 `null` (explicit unsupported).
    pub counters: Option<WorkerCounters>,
    /// Per-worker throughput; v1 `null`.
    pub throughput: Option<u64>,
    /// Explicit support marker for `throughput` (`unsupported` in v1).
    pub throughput_support: Option<String>,
    /// Per-worker drops; v1 `null`.
    pub drops: Option<u64>,
    /// Explicit support marker for `drops` (`unsupported` in v1).
    pub drops_support: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, ToSchema)]
pub struct WorkerCounters {
    pub rx_packets: u64,
    pub tx_packets: u64,
    pub drop_packets: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, ToSchema)]
pub struct OwnershipSection {
    pub available: bool,
    pub reason: Option<String>,
    pub generation: Option<String>,
    pub circuits: Option<Vec<CircuitOwner>>,
    pub nodes: Option<Vec<NodeOwners>>,
    pub crossings: Option<Vec<String>>,
    /// Circuits whose owner is the engine's `u16::MAX` sentinel.
    pub unowned_circuits: Option<u32>,
}

impl OwnershipSection {
    fn unavailable(reason: &str) -> Self {
        Self {
            available: false,
            reason: Some(reason.to_string()),
            generation: None,
            circuits: None,
            nodes: None,
            crossings: None,
            unowned_circuits: None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, ToSchema)]
pub struct CircuitOwner {
    pub circuit_id: String,
    pub parent_node_id: String,
    /// Operator-facing display name, or `null` when absent (never empty).
    pub name: Option<String>,
    /// `null` = the circuit has no owner (the engine sentinel), never 65535/0.
    pub worker_id: Option<u16>,
}

#[derive(Serialize, Deserialize, Clone, Debug, ToSchema)]
pub struct NodeOwners {
    pub node_id: String,
    pub parent_node_id: Option<String>,
    /// Operator-facing display name, or `null` when absent (never empty).
    pub name: Option<String>,
    pub worker_ids: Vec<u16>,
    pub crossing: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, ToSchema)]
pub struct MovesSection {
    pub available: bool,
    pub reason: Option<String>,
    pub source: Option<String>,
    pub since_unix: Option<u64>,
    pub items: Option<Vec<MoveRow>>,
}

impl MovesSection {
    fn unavailable(reason: &str) -> Self {
        Self {
            available: false,
            reason: Some(reason.to_string()),
            source: None,
            since_unix: None,
            items: None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, ToSchema)]
pub struct MoveRow {
    pub circuit_id: String,
    pub from_worker: Option<u16>,
    pub to_worker: Option<u16>,
    pub generation: Option<String>,
    pub observed_at_unix: Option<u64>,
    pub source: Option<String>,
    pub cause: Option<String>,
}

/// The bus wire mirror produced by lqosd (`GetCpuWorkers`). Kept typed here so
/// the OpenAPI contract is the only public schema; the bus itself carries the
/// versioned JSON document (the `ConvergenceState` precedent).
#[derive(Deserialize, Clone, Debug)]
pub struct CpuWorkersBus {
    #[allow(dead_code)]
    pub schema: u32,
    pub generated_at_unix: u64,
    pub bridge: BridgeSection,
    pub workers: WorkersSection,
    pub ownership: OwnershipSection,
    pub moves: MovesSection,
}

/// Record one unavailable section's reason, deduped. A section that is present
/// contributes nothing (even when it carries an informational reason).
fn push_unavailable_reason(
    reasons: &mut Vec<String>,
    available: bool,
    reason: &Option<String>,
    fallback: &str,
) {
    if available {
        return;
    }
    let reason = reason.clone().unwrap_or_else(|| fallback.to_string());
    if !reasons.contains(&reason) {
        reasons.push(reason);
    }
}

/// Assemble the API response from the local CPU sources and the optional bus
/// snapshot. Pure and unit-testable; never fabricates a value for an absent
/// source.
pub fn assemble_response(
    generated_at_unix: u64,
    cpu_usage: CpuUsageSection,
    cpu_topology: CpuTopologySection,
    bus: Option<CpuWorkersBus>,
) -> CpuWorkersResponse {
    let (bridge, workers, ownership, moves) = match bus {
        Some(bus) => (bus.bridge, bus.workers, bus.ownership, bus.moves),
        None => (
            BridgeSection::unavailable("bus_unavailable"),
            WorkersSection::unavailable("bus_unavailable"),
            OwnershipSection::unavailable("bus_unavailable"),
            MovesSection::unavailable("bus_unavailable"),
        ),
    };

    let mut unavailable_reasons = Vec::new();
    push_unavailable_reason(
        &mut unavailable_reasons,
        cpu_usage.available,
        &cpu_usage.reason,
        "cpu_usage_unavailable",
    );
    push_unavailable_reason(
        &mut unavailable_reasons,
        cpu_topology.available,
        &cpu_topology.reason,
        "cpu_topology_unavailable",
    );
    push_unavailable_reason(
        &mut unavailable_reasons,
        bridge.available,
        &bridge.reason,
        "bridge_unavailable",
    );
    push_unavailable_reason(
        &mut unavailable_reasons,
        workers.available,
        &workers.reason,
        "workers_unavailable",
    );
    push_unavailable_reason(
        &mut unavailable_reasons,
        ownership.available,
        &ownership.reason,
        "ownership_unavailable",
    );
    push_unavailable_reason(
        &mut unavailable_reasons,
        moves.available,
        &moves.reason,
        "moves_unavailable",
    );

    let all_available = cpu_usage.available
        && cpu_topology.available
        && bridge.available
        && workers.available
        && ownership.available
        && moves.available;
    let any_available = cpu_usage.available
        || cpu_topology.available
        || bridge.available
        || workers.available
        || ownership.available
        || moves.available;
    let state = if all_available {
        "ok"
    } else if any_available {
        "partial"
    } else {
        "unavailable"
    };

    CpuWorkersResponse {
        state: state.to_string(),
        unavailable_reasons,
        generated_at_unix,
        cpu_usage,
        cpu_topology,
        bridge,
        workers,
        ownership,
        moves,
    }
}

fn cpu_usage_section(stats: SystemUsageSnapshot) -> CpuUsageSection {
    if stats.num_cpus == 0 {
        // The daemon's sampler has not produced its first tick yet: unavailable
        // is honest, an empty or zero row list is not.
        return CpuUsageSection {
            available: false,
            reason: Some("system_monitor_unavailable".to_string()),
            num_cpus: None,
            per_cpu: None,
        };
    }
    if stats.cpu_usage.len() != stats.cpu_ids.len() {
        return CpuUsageSection {
            available: false,
            reason: Some("cpu_id_mapping_unproven".to_string()),
            num_cpus: Some(stats.num_cpus),
            per_cpu: None,
        };
    }
    let per_cpu: Vec<CpuUsageRow> = stats
        .cpu_usage
        .iter()
        .zip(stats.cpu_ids.iter())
        .map(|(percent, cpu_id)| CpuUsageRow {
            cpu_id: if *cpu_id == u32::MAX {
                None
            } else {
                Some(*cpu_id)
            },
            percent: *percent,
        })
        .collect();
    CpuUsageSection {
        available: true,
        reason: None,
        num_cpus: Some(stats.num_cpus),
        per_cpu: Some(per_cpu),
    }
}

async fn fetch_bus() -> Option<CpuWorkersBus> {
    let mut client = LibreqosBusClient::new().await.ok()?;
    let responses = client.request(vec![BusRequest::GetCpuWorkers]).await.ok()?;
    for response in responses {
        if let BusResponse::CpuWorkers(document) = response {
            return serde_json::from_str::<CpuWorkersBus>(&document).ok();
        }
    }
    None
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// `GET /cpu_workers` — read-only CPU topology + bridge worker/ownership
/// observability. A missing bus is a 200 `partial` (CPU is still useful), not
/// a 500.
#[utoipa::path(
    get,
    path = "/cpu_workers",
    summary = "CPU topology, bridge workers, ownership and placements",
    description = "Read-only CPU & Workers view: per-CPU usage, shaping-CPU topology, bridge worker affinity (lcore/NUMA/queue ids), applied circuit/node ownership sets and crossings, bridge placement state, and moves availability. Absent sources are explicit `available:false` + `reason` + `null` payloads; counters/throughput/drops are explicitly unsupported in v1. No kernel cpu_affinity surface.",
    responses(
        (status = 200, description = "CPU & Workers snapshot (possibly partial with explicit unavailable sections).", body = CpuWorkersResponse),
        (status = 401, description = "Unauthorized", body = String),
        (status = 403, description = "Forbidden", body = String),
    ),
    tag = "CPU & Workers",
    params(
        ("x-bearer" = String, Header, description = "Named or legacy local API key, or license key.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn cpu_workers(
    State(state): State<Arc<ApiState>>,
) -> (StatusCode, Json<CpuWorkersResponse>) {
    let bus = fetch_bus().await;
    let response = assemble_response(
        now_unix(),
        cpu_usage_section(state.system_stats()),
        state.cpu_topology(),
        bus,
    );
    (StatusCode::OK, Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cpu() -> CpuUsageSection {
        CpuUsageSection {
            available: true,
            reason: None,
            num_cpus: Some(1),
            per_cpu: Some(vec![CpuUsageRow {
                cpu_id: Some(0),
                percent: 12,
            }]),
        }
    }

    fn topology() -> CpuTopologySection {
        CpuTopologySection {
            available: true,
            reason: None,
            exclude_efficiency_cores: Some(false),
            source: Some("exclusion_disabled".to_string()),
            from_cache: Some(false),
            has_hybrid_split: Some(false),
            possible: Some(vec![0]),
            performance: Some(vec![0]),
            efficiency: Some(vec![]),
            shaping: Some(vec![0]),
            detail: Some("Efficiency-core exclusion disabled".to_string()),
        }
    }

    fn bus(workers: WorkersSection, ownership: OwnershipSection) -> CpuWorkersBus {
        CpuWorkersBus {
            schema: 1,
            generated_at_unix: 1,
            bridge: BridgeSection::unavailable("no_bridge_report"),
            workers,
            ownership,
            moves: MovesSection::unavailable("no_history_source_v1"),
        }
    }

    #[test]
    fn absent_bus_is_partial_and_never_zeros() {
        let response = assemble_response(1, cpu(), topology(), None);
        assert_eq!(response.state, "partial");
        assert!(!response.workers.available);
        assert!(response.workers.rows.is_none());
        assert!(!response.ownership.available);
        assert!(response.ownership.circuits.is_none());
        assert!(!response.moves.available);
        assert!(response.moves.items.is_none());
    }

    #[test]
    fn present_empty_workers_is_supported_empty() {
        let workers = WorkersSection {
            available: true,
            reason: None,
            schema: Some(1),
            rx_queue_count: Some(0),
            tx_queue_count: Some(0),
            counter_support: Some("unsupported".to_string()),
            rows: Some(vec![]),
        };
        let ownership = OwnershipSection {
            available: true,
            reason: None,
            generation: Some("g".to_string()),
            circuits: Some(vec![]),
            nodes: Some(vec![]),
            crossings: Some(vec![]),
            unowned_circuits: Some(0),
        };
        let response = assemble_response(1, cpu(), topology(), Some(bus(workers, ownership)));
        assert_eq!(response.state, "partial", "moves are unavailable in v1");
        assert!(response.workers.available);
        assert!(
            response
                .workers
                .rows
                .as_deref()
                .is_some_and(|rows| rows.is_empty())
        );
    }

    #[test]
    fn unowned_circuit_is_null_not_65535() {
        let ownership = OwnershipSection {
            available: true,
            reason: None,
            generation: Some("g".to_string()),
            circuits: Some(vec![CircuitOwner {
                circuit_id: "c1".to_string(),
                parent_node_id: "root".to_string(),
                name: None,
                worker_id: None,
            }]),
            nodes: Some(vec![]),
            crossings: Some(vec![]),
            unowned_circuits: Some(1),
        };
        let workers = WorkersSection {
            available: true,
            reason: None,
            schema: Some(1),
            rx_queue_count: Some(1),
            tx_queue_count: Some(1),
            counter_support: Some("unsupported".to_string()),
            rows: Some(vec![WorkerRow {
                worker_id: 0,
                lcore_id: None,
                numa_node: None,
                rx_queue_ids: None,
                tx_queue_id: None,
                shared_tx_queue: None,
                circuit_count: Some(0),
                solely_owned_node_count: Some(0),
                counters: None,
                throughput: None,
                throughput_support: Some("unsupported".to_string()),
                drops: None,
                drops_support: Some("unsupported".to_string()),
            }]),
        };
        let response = assemble_response(1, cpu(), topology(), Some(bus(workers, ownership)));
        let ownership = response.ownership;
        assert_eq!(ownership.unowned_circuits, Some(1));
        assert_eq!(
            ownership.circuits.unwrap()[0].worker_id,
            None,
            "the u16::MAX sentinel must never leak as a worker id"
        );
        let rows = response.workers.rows.unwrap();
        let worker = &rows[0];
        assert!(worker.counters.is_none());
        assert!(worker.throughput.is_none());
        assert!(worker.drops.is_none());
    }

    #[test]
    fn underivable_cpu_ids_are_null() {
        let section = cpu_usage_section(SystemUsageSnapshot {
            cpu_usage: vec![5],
            cpu_ids: vec![u32::MAX],
            num_cpus: 1,
            ram_used: 1,
            total_ram: 2,
        });
        assert_eq!(section.per_cpu.unwrap()[0].cpu_id, None);
    }

    #[test]
    fn topology_source_uses_explicit_snake_case() {
        assert_eq!(
            topology_source_str(lqos_config::ShapingCpuSource::ExclusionDisabled),
            "exclusion_disabled"
        );
        assert_eq!(
            topology_source_str(lqos_config::ShapingCpuSource::CoreTypeSysfs),
            "core_type_sysfs"
        );
        assert_eq!(
            topology_source_str(lqos_config::ShapingCpuSource::PossibleMinusAtomSysfs),
            "possible_minus_atom_sysfs"
        );
        assert_eq!(
            topology_source_str(lqos_config::ShapingCpuSource::FallbackAvailableParallelism),
            "fallback_available_parallelism"
        );
    }

    /// Cold start: the daemon's sampler has not ticked, so the section says so
    /// instead of reporting an empty CPU list as a measurement.
    #[test]
    fn an_unticked_sampler_is_unavailable_not_empty() {
        let section = cpu_usage_section(SystemUsageSnapshot {
            cpu_usage: Vec::new(),
            cpu_ids: Vec::new(),
            num_cpus: 0,
            ram_used: 0,
            total_ram: 0,
        });
        assert!(!section.available);
        assert_eq!(
            section.reason.as_deref(),
            Some("system_monitor_unavailable")
        );
        assert!(section.per_cpu.is_none());
    }

    #[test]
    fn cpu_usage_length_mismatch_is_unavailable() {
        let section = cpu_usage_section(SystemUsageSnapshot {
            cpu_usage: vec![5, 6],
            cpu_ids: vec![0],
            num_cpus: 2,
            ram_used: 1,
            total_ram: 2,
        });
        assert!(!section.available);
        assert_eq!(section.reason.as_deref(), Some("cpu_id_mapping_unproven"));
        assert!(section.per_cpu.is_none());
    }
}
