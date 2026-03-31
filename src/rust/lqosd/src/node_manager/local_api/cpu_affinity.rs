use crate::node_manager::local_api::network_tree_lite::{
    NetworkTreeLiteNode, network_tree_lite_data,
};
use crate::shaped_devices_tracker::SHAPED_DEVICES;
use crate::system_stats::{CPU_USAGE, NUM_CPUS};
use lqos_config::{ShapingCpuDetection, detect_shaping_cpus, load_config};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Clone, Debug)]
pub struct CpuSideSummary {
    pub circuits: usize,
    pub min_sum_mbps: f64,
    pub max_sum_mbps: f64,
    pub weight_sum: f64,
}

#[derive(Serialize, Clone, Debug)]
pub struct CpuAffinitySummaryEntry {
    pub cpu: u32,
    pub download: CpuSideSummary,
    pub upload: CpuSideSummary,
}

#[derive(Serialize, Clone, Debug)]
pub struct CircuitBrief {
    pub circuit_id: Option<String>,
    pub circuit_name: Option<String>,
    pub parent_node: Option<String>,
    pub classid: Option<String>,
    pub max_mbps: f64,
    pub weight: f64,
    pub ip_count: usize,
    pub ignored: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct CpuAffinityCircuitsPage {
    pub cpu: u32,
    pub direction: String,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
    pub items: Vec<CircuitBrief>,
}

#[derive(Serialize, Clone, Debug)]
pub struct CpuAffinitySiteTreeNode {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<CpuAffinitySiteTreeNode>,
}

/// Runtime-aware CPU affinity snapshot for the CPU assignment page.
#[derive(Serialize, Clone, Debug)]
pub struct CpuAffinityRuntimeSnapshot {
    /// Snapshot generation time in milliseconds since the UNIX epoch.
    pub generated_at_unix_ms: u64,
    /// CPU IDs currently intended for shaping / binning.
    pub shaping_cpus: Vec<u32>,
    /// CPU IDs present on the host but excluded from shaping.
    pub excluded_cpus: Vec<u32>,
    /// True when hybrid-core detection found a trustworthy split.
    pub has_hybrid_split: bool,
    /// Per-core runtime assignment data.
    pub cores: Vec<CpuAffinityRuntimeCore>,
}

/// Runtime-aware data for a single CPU core.
#[derive(Serialize, Clone, Debug)]
pub struct CpuAffinityRuntimeCore {
    /// Zero-based CPU index.
    pub cpu: u32,
    /// Live CPU usage for this core, if sampled.
    pub live_usage_pct: Option<u32>,
    /// Number of planned download-side circuits on this CPU.
    pub planned_circuit_count: u32,
    /// Sum of planner weights for download-side circuits on this CPU.
    pub planned_weight_sum: f64,
    /// Sum of planned download-side max rates for this CPU.
    pub planned_max_mbps: f64,
    /// Number of nodes effectively associated with this CPU.
    pub effective_node_count: u32,
    /// Number of effective site nodes associated with this CPU.
    pub effective_site_count: u32,
    /// Number of effective AP nodes associated with this CPU.
    pub effective_ap_count: u32,
    /// Number of effective circuits associated with this CPU.
    pub effective_circuit_count: u32,
    /// Number of runtime-changed nodes associated with this CPU.
    pub runtime_changed_count: u32,
    /// Detailed node rows associated with this CPU.
    pub nodes: Vec<CpuAffinityRuntimeNode>,
}

/// Runtime-aware node assignment details for a single site/AP branch.
#[derive(Serialize, Clone, Debug)]
pub struct CpuAffinityRuntimeNode {
    /// Node name from the live tree.
    pub name: String,
    /// Optional node type metadata.
    pub node_type: Option<String>,
    /// Planned CPU from queue generation, if known.
    pub planned_cpu: Option<u32>,
    /// Effective CPU after runtime virtualization, if known.
    pub effective_cpu: Option<u32>,
    /// True if TreeGuard has runtime-virtualized this node.
    pub runtime_virtualized: bool,
    /// Assignment reason used by the UI to explain runtime changes.
    pub assignment_reason: String,
    /// True when this node is a top-level owned branch directly attached to the
    /// selected CPU's HTB subtree, rather than a descendant of another owned branch.
    pub is_cpu_root: bool,
    /// Number of descendant nodes beneath this node.
    pub subtree_node_count: u32,
    /// Number of descendant circuits beneath this node.
    pub subtree_circuit_count: u32,
    /// Effective max throughput in Mbps for down/up from the live tree.
    pub effective_max_mbps: (f64, f64),
    /// Current throughput in bytes per second, down/up.
    pub current_throughput_bps: (u64, u64),
}

#[derive(Debug, Default, Clone)]
struct CircuitRecord {
    cpu_down: Option<u32>,
    cpu_up: Option<u32>,
    min_down: f64,
    max_down: f64,
    min_up: f64,
    max_up: f64,
    planner_weight: f64,
    has_planner_weight: bool,
    classid_down: Option<String>,
    classid_up: Option<String>,
    circuit_id: Option<String>,
    circuit_name: Option<String>,
    parent_node: Option<String>,
    ip_count: usize,
}

#[derive(Debug, Default, Clone, Copy)]
struct PlannedCoreMetrics {
    circuit_count: u32,
    weight_sum: f64,
    max_mbps: f64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct RuntimeNodePlacement {
    owner_index: Option<usize>,
    planned_cpu: Option<u32>,
    effective_cpu: Option<u32>,
    assignment_reason: &'static str,
}

// Query params for the circuits endpoint
#[derive(Debug, Default, Clone, serde::Deserialize)]
pub struct CircuitsQuery {
    pub direction: Option<String>, // "down" | "up" (default: down)
    pub page: Option<usize>,       // default 1
    pub page_size: Option<usize>,  // default 100
    pub search: Option<String>,    // substring match on id/name
}

fn queuing_structure_path() -> Option<PathBuf> {
    let cfg = lqos_config::load_config().ok()?;
    let mut p = PathBuf::from(cfg.lqos_directory.clone());
    p.push("queuingStructure.json");
    Some(p)
}

fn parse_hex_u32(s: &str) -> Option<u32> {
    // Accept formats like "0x1" or decimal strings
    if let Some(stripped) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(stripped, 16).ok()
    } else {
        s.parse::<u32>().ok()
    }
}

fn val_as_f64(v: &Value) -> Option<f64> {
    if let Some(x) = v.as_f64() {
        Some(x)
    } else if let Some(x) = v.as_i64() {
        Some(x as f64)
    } else if let Some(x) = v.as_u64() {
        Some(x as f64)
    } else if let Some(s) = v.as_str() {
        s.parse::<f64>().ok()
    } else {
        None
    }
}

fn collect_ip_count(circuit: &serde_json::Map<String, Value>) -> usize {
    let mut count = 0usize;
    if let Some(Value::Array(devs)) = circuit.get("devices") {
        for d in devs {
            if let Value::Object(dev) = d {
                if let Some(Value::Array(v4s)) = dev.get("ipv4s") {
                    count += v4s.len();
                }
                if let Some(Value::Array(v6s)) = dev.get("ipv6s") {
                    count += v6s.len();
                }
            }
        }
    }
    count
}

fn add_circuit_records(
    parent_name: Option<&str>,
    node: &serde_json::Map<String, Value>,
    out: &mut Vec<CircuitRecord>,
) {
    // Helper: parse a JSON object that represents a circuit-level entry
    fn parse_circuit_map(
        parent_name: Option<&str>,
        cm: &serde_json::Map<String, Value>,
    ) -> CircuitRecord {
        let mut rec = CircuitRecord::default();
        // cpu numbers
        if let Some(Value::String(s)) = cm.get("cpuNum") {
            rec.cpu_down = parse_hex_u32(s);
        } else if let Some(Value::String(s)) = cm.get("classMajor") {
            if let Some(v) = parse_hex_u32(s) {
                rec.cpu_down = Some(v.saturating_sub(1));
            }
        } else if let Some(Value::String(s)) = cm.get("classid") {
            // Fallback: parse major from TC handle like "0x1:0x51"
            if let Some((maj, _)) = s.split_once(':')
                && let Some(v) = parse_hex_u32(maj)
            {
                rec.cpu_down = Some(v.saturating_sub(1));
            }
        }
        if let Some(Value::String(s)) = cm.get("up_cpuNum") {
            rec.cpu_up = parse_hex_u32(s);
        } else if let Some(Value::String(s)) = cm.get("up_classMajor") {
            if let Some(v) = parse_hex_u32(s) {
                rec.cpu_up = Some(v.saturating_sub(1));
            }
        } else if let Some(Value::String(s)) = cm.get("up_classid")
            && let Some((maj, _)) = s.split_once(':')
            && let Some(v) = parse_hex_u32(maj)
        {
            rec.cpu_up = Some(v.saturating_sub(1));
        }
        // classids
        if let Some(Value::String(s)) = cm.get("classid") {
            rec.classid_down = Some(s.clone());
        }
        if let Some(Value::String(s)) = cm.get("up_classid") {
            rec.classid_up = Some(s.clone());
        }
        // bandwidths
        if let Some(v) = cm.get("minDownload").and_then(val_as_f64) {
            rec.min_down = v;
        }
        if let Some(v) = cm.get("maxDownload").and_then(val_as_f64) {
            rec.max_down = v;
        }
        if let Some(v) = cm.get("minUpload").and_then(val_as_f64) {
            rec.min_up = v;
        }
        if let Some(v) = cm.get("maxUpload").and_then(val_as_f64) {
            rec.max_up = v;
        }
        // planner weight if present
        if let Some(v) = cm.get("planner_weight").and_then(val_as_f64) {
            rec.planner_weight = v;
            rec.has_planner_weight = true;
        }
        // identity
        if let Some(Value::String(s)) = cm.get("circuitID")
            && !s.is_empty()
        {
            rec.circuit_id = Some(s.clone());
        }
        if let Some(Value::String(s)) = cm.get("circuitName")
            && !s.is_empty()
        {
            rec.circuit_name = Some(s.clone());
        }
        if rec.circuit_name.is_none() {
            // Fallback to the JSON key is not available here; leave empty
        }
        rec.parent_node = parent_name.map(|x| x.to_string());
        // IP count
        rec.ip_count = collect_ip_count(cm);
        rec
    }

    // Process circuits array at this node
    if let Some(Value::Array(circuits)) = node.get("circuits") {
        for c in circuits {
            if let Value::Object(cm) = c {
                out.push(parse_circuit_map(parent_name, cm));
            }
        }
    }

    // Also check children: some builds embed circuits directly as child entries (rare).
    if let Some(Value::Object(children)) = node.get("children") {
        for (child_name, child_value) in children.iter() {
            if let Value::Object(child_map) = child_value {
                // Treat as circuit only if it has a devices array (avoids counting structural nodes)
                let is_circuit_like = matches!(child_map.get("devices"), Some(Value::Array(_)))
                    || child_map.get("circuitID").is_some();
                if is_circuit_like {
                    out.push(parse_circuit_map(Some(child_name.as_str()), child_map));
                }
                add_circuit_records(Some(child_name.as_str()), child_map, out);
            }
        }
    }
}

fn load_all_circuits() -> Vec<CircuitRecord> {
    let Some(path) = queuing_structure_path() else {
        return Vec::new();
    };
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let json: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut circuits = Vec::new();
    if let Value::Object(map) = json
        && let Some(Value::Object(net)) = map.get("Network")
    {
        for (_k, v) in net.iter() {
            if let Value::Object(node) = v {
                add_circuit_records(None, node, &mut circuits);
            }
        }
    }
    circuits
}

fn trailing_u32(s: &str) -> Option<u32> {
    let digits: String = s
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<Vec<char>>()
        .into_iter()
        .rev()
        .collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse::<u32>().ok()
    }
}

fn node_cpu_from_queue_map(
    node: &serde_json::Map<String, Value>,
    inherited_cpu: Option<u32>,
) -> Option<u32> {
    node.get("cpuNum")
        .and_then(|v| v.as_str())
        .and_then(parse_hex_u32)
        .or_else(|| {
            node.get("classMajor")
                .and_then(|v| v.as_str())
                .and_then(parse_hex_u32)
                .map(|v| v.saturating_sub(1))
        })
        .or(inherited_cpu)
}

fn collect_planned_node_cpus(
    name: &str,
    node: &serde_json::Map<String, Value>,
    inherited_cpu: Option<u32>,
    out: &mut HashMap<String, u32>,
) {
    if !is_network_node(node) {
        return;
    }

    let cpu = node_cpu_from_queue_map(node, inherited_cpu);
    if let Some(cpu) = cpu {
        out.insert(name.to_string(), cpu);
    }

    if let Some(Value::Object(children)) = node.get("children") {
        for (child_name, child_value) in children {
            if let Value::Object(child_map) = child_value {
                collect_planned_node_cpus(child_name, child_map, cpu, out);
            }
        }
    }
}

fn load_planned_node_cpu_map() -> HashMap<String, u32> {
    let Some(path) = queuing_structure_path() else {
        return HashMap::new();
    };
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return HashMap::new(),
    };
    let json: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return HashMap::new(),
    };
    let Some(net) = json.get("Network").and_then(|v| v.as_object()) else {
        return HashMap::new();
    };

    let looks_binned = net.keys().any(|k| k.starts_with("CpueQueue"));
    let mut out = HashMap::new();

    if looks_binned {
        for (cpu_key, cpu_value) in net {
            let Some(cpu_node) = cpu_value.as_object() else {
                continue;
            };
            let cpu = trailing_u32(cpu_key)
                .or_else(|| node_cpu_from_queue_map(cpu_node, None))
                .unwrap_or_default();
            if let Some(Value::Object(children)) = cpu_node.get("children") {
                for (child_name, child_value) in children {
                    if let Value::Object(child_map) = child_value {
                        collect_planned_node_cpus(child_name, child_map, Some(cpu), &mut out);
                    }
                }
            }
        }
    } else {
        for (child_name, child_value) in net {
            let Some(child_map) = child_value.as_object() else {
                continue;
            };
            collect_planned_node_cpus(child_name, child_map, None, &mut out);
        }
    }

    out
}

fn owner_index_for_node(nodes: &[(usize, NetworkTreeLiteNode)], idx: usize) -> Option<usize> {
    if idx == 0 || idx >= nodes.len() {
        return None;
    }

    let node = &nodes[idx].1;
    if node.runtime_virtualized {
        let mut current = node.immediate_parent;
        while let Some(parent_idx) = current {
            if parent_idx >= nodes.len() {
                return None;
            }
            let parent = &nodes[parent_idx].1;
            if !parent.runtime_virtualized {
                return Some(parent_idx);
            }
            current = parent.immediate_parent;
        }
        return None;
    }

    let mut current = node.immediate_parent;
    let mut saw_runtime_virtualized_ancestor = false;
    while let Some(parent_idx) = current {
        if parent_idx >= nodes.len() {
            return None;
        }
        let parent = &nodes[parent_idx].1;
        if parent.runtime_virtualized {
            saw_runtime_virtualized_ancestor = true;
            current = parent.immediate_parent;
            continue;
        }
        if saw_runtime_virtualized_ancestor {
            return Some(parent_idx);
        }
        current = parent.immediate_parent;
    }

    Some(idx)
}

fn fallback_effective_cpu(
    nodes: &[(usize, NetworkTreeLiteNode)],
    planned_cpu_by_name: &HashMap<String, u32>,
    idx: usize,
) -> Option<u32> {
    let mut current = Some(idx);
    while let Some(node_idx) = current {
        if node_idx >= nodes.len() {
            return None;
        }
        if let Some(cpu) = planned_cpu_by_name
            .get(nodes[node_idx].1.name.as_str())
            .copied()
        {
            return Some(cpu);
        }
        current = nodes[node_idx].1.immediate_parent;
    }
    None
}

fn derive_runtime_node_placements(
    nodes: &[(usize, NetworkTreeLiteNode)],
    planned_cpu_by_name: &HashMap<String, u32>,
) -> Vec<RuntimeNodePlacement> {
    let mut placements = Vec::with_capacity(nodes.len());

    for idx in 0..nodes.len() {
        if idx == 0 {
            placements.push(RuntimeNodePlacement::default());
            continue;
        }

        let planned_cpu = planned_cpu_by_name.get(nodes[idx].1.name.as_str()).copied();
        let owner_index = owner_index_for_node(nodes, idx);
        let effective_cpu = owner_index
            .and_then(|owner| {
                if owner == 0 {
                    None
                } else {
                    planned_cpu_by_name
                        .get(nodes[owner].1.name.as_str())
                        .copied()
                }
            })
            .or_else(|| fallback_effective_cpu(nodes, planned_cpu_by_name, idx));

        let assignment_reason = if nodes[idx].1.runtime_virtualized {
            "runtime_virtualized_hidden"
        } else if owner_index.is_some_and(|owner| owner != idx) {
            "inherited_from_virtualized_ancestor"
        } else if effective_cpu.is_some() {
            "planned"
        } else {
            "unknown"
        };

        placements.push(RuntimeNodePlacement {
            owner_index,
            planned_cpu,
            effective_cpu,
            assignment_reason,
        });
    }

    placements
}

fn direct_circuit_counts_by_node() -> HashMap<String, u32> {
    let shaped = SHAPED_DEVICES.load();
    let mut circuits_by_node: HashMap<String, BTreeSet<i64>> = HashMap::new();

    for device in &shaped.devices {
        let node_name = device.parent_node.trim();
        if node_name.is_empty() {
            continue;
        }
        circuits_by_node
            .entry(node_name.to_string())
            .or_default()
            .insert(device.circuit_hash);
    }

    circuits_by_node
        .into_iter()
        .map(|(name, circuits)| (name, circuits.len() as u32))
        .collect()
}

fn build_subtree_counts(
    nodes: &[(usize, NetworkTreeLiteNode)],
    direct_circuit_counts: &HashMap<String, u32>,
) -> (Vec<u32>, Vec<u32>) {
    let mut subtree_node_counts = vec![0u32; nodes.len()];
    let mut subtree_circuit_counts = vec![0u32; nodes.len()];

    for (idx, (_, node)) in nodes.iter().enumerate() {
        subtree_circuit_counts[idx] = direct_circuit_counts
            .get(node.name.as_str())
            .copied()
            .unwrap_or(0);
    }

    for idx in (1..nodes.len()).rev() {
        let Some(parent_idx) = nodes[idx].1.immediate_parent else {
            continue;
        };
        if parent_idx >= nodes.len() {
            continue;
        }
        subtree_node_counts[parent_idx] = subtree_node_counts[parent_idx]
            .saturating_add(1)
            .saturating_add(subtree_node_counts[idx]);
        subtree_circuit_counts[parent_idx] =
            subtree_circuit_counts[parent_idx].saturating_add(subtree_circuit_counts[idx]);
    }

    (subtree_node_counts, subtree_circuit_counts)
}

fn is_cpu_root_node(
    nodes: &[(usize, NetworkTreeLiteNode)],
    placements: &[RuntimeNodePlacement],
    idx: usize,
) -> bool {
    if idx == 0 || idx >= nodes.len() || idx >= placements.len() {
        return false;
    }

    let placement = &placements[idx];
    let Some(effective_cpu) = placement.effective_cpu else {
        return false;
    };
    if nodes[idx].1.runtime_virtualized {
        return false;
    }

    let mut current = nodes[idx].1.immediate_parent;
    let mut saw_runtime_virtualized_ancestor = false;
    while let Some(parent_idx) = current {
        if parent_idx == 0 || parent_idx >= nodes.len() {
            break;
        }
        let parent = &nodes[parent_idx].1;
        if parent.runtime_virtualized {
            saw_runtime_virtualized_ancestor = true;
            current = parent.immediate_parent;
            continue;
        }
        break;
    }

    if saw_runtime_virtualized_ancestor {
        return true;
    }

    if placement.owner_index != Some(idx) {
        return false;
    }

    let Some(parent_idx) = nodes[idx].1.immediate_parent else {
        return true;
    };
    if parent_idx == 0 || parent_idx >= placements.len() {
        return true;
    }

    placements[parent_idx].effective_cpu != Some(effective_cpu)
}

fn aggregate_planned_core_metrics(circuits: &[CircuitRecord]) -> HashMap<u32, PlannedCoreMetrics> {
    let mut planned = HashMap::new();

    for circuit in circuits {
        let ignored = circuit.has_planner_weight && circuit.planner_weight <= 0.0;
        if ignored {
            continue;
        }
        let Some(cpu) = circuit.cpu_down else {
            continue;
        };
        let entry = planned
            .entry(cpu)
            .or_insert_with(PlannedCoreMetrics::default);
        entry.circuit_count = entry.circuit_count.saturating_add(1);
        entry.weight_sum += circuit.planner_weight;
        entry.max_mbps += circuit.max_down;
    }

    planned
}

fn live_cpu_usage_by_index() -> HashMap<u32, u32> {
    let count = NUM_CPUS.load(std::sync::atomic::Ordering::Relaxed);
    let mut usage = HashMap::new();
    for cpu in 0..count {
        usage.insert(
            cpu as u32,
            CPU_USAGE[cpu].load(std::sync::atomic::Ordering::Relaxed),
        );
    }
    usage
}

fn inferred_included_cpus(
    planned_core_metrics: &HashMap<u32, PlannedCoreMetrics>,
    placements: &[RuntimeNodePlacement],
) -> BTreeSet<u32> {
    let mut included = BTreeSet::new();

    for cpu in planned_core_metrics.keys().copied() {
        included.insert(cpu);
    }

    for placement in placements {
        if let Some(cpu) = placement.planned_cpu {
            included.insert(cpu);
        }
        if let Some(cpu) = placement.effective_cpu {
            included.insert(cpu);
        }
    }

    included
}

fn resolve_snapshot_cpu_sets(
    detection: Option<ShapingCpuDetection>,
    all_cpus: &BTreeSet<u32>,
    planned_core_metrics: &HashMap<u32, PlannedCoreMetrics>,
    placements: &[RuntimeNodePlacement],
) -> (Vec<u32>, Vec<u32>, bool) {
    let Some(detection) = detection else {
        return (Vec::new(), Vec::new(), false);
    };

    let possible = if detection.possible.is_empty() {
        all_cpus.clone()
    } else {
        detection.possible.iter().copied().collect::<BTreeSet<_>>()
    };

    if detection.has_hybrid_split {
        let shaping = detection.shaping.into_iter().collect::<BTreeSet<_>>();
        let excluded = possible.difference(&shaping).copied().collect::<Vec<_>>();
        return (shaping.into_iter().collect(), excluded, true);
    }

    if detection.exclude_efficiency_cores {
        let inferred = inferred_included_cpus(planned_core_metrics, placements);
        if !inferred.is_empty() && inferred.len() < possible.len() {
            let excluded = possible.difference(&inferred).copied().collect::<Vec<_>>();
            return (inferred.into_iter().collect(), excluded, false);
        }
    }

    let shaping = detection.shaping;
    let excluded = possible
        .difference(&shaping.iter().copied().collect::<BTreeSet<_>>())
        .copied()
        .collect::<Vec<_>>();
    (shaping, excluded, detection.has_hybrid_split)
}

fn is_site_type(node_type: Option<&str>) -> bool {
    node_type.is_some_and(|t| t.eq_ignore_ascii_case("site"))
}

fn is_ap_type(node_type: Option<&str>) -> bool {
    node_type.is_some_and(|t| t.eq_ignore_ascii_case("ap"))
}

fn is_network_node(node: &serde_json::Map<String, Value>) -> bool {
    // Prefer explicit type checks where present
    if node
        .get("type")
        .and_then(|v| v.as_str())
        .is_some_and(|t| t.eq_ignore_ascii_case("site") || t.eq_ignore_ascii_case("ap"))
    {
        return true;
    }

    // Some production `network.json` files omit `type`. In `queuingStructure.json`,
    // network nodes always include bandwidth keys (circuits do not).
    node.contains_key("downloadBandwidthMbps")
        || node.contains_key("uploadBandwidthMbps")
        || node.contains_key("children")
}

fn build_site_tree(name: &str, node: &serde_json::Map<String, Value>) -> CpuAffinitySiteTreeNode {
    let mut children = Vec::new();
    if let Some(Value::Object(child_map)) = node.get("children") {
        for (child_name, child_value) in child_map.iter() {
            if let Value::Object(child_node) = child_value
                && is_network_node(child_node)
            {
                children.push(build_site_tree(child_name, child_node));
            }
        }
    }
    children.sort_by(|a, b| a.name.cmp(&b.name));
    CpuAffinitySiteTreeNode {
        name: name.to_string(),
        children,
    }
}

pub fn cpu_affinity_site_tree_data() -> Option<CpuAffinitySiteTreeNode> {
    let path = queuing_structure_path()?;
    let raw = std::fs::read_to_string(path).ok()?;
    let json: Value = serde_json::from_str(&raw).ok()?;
    let net = json.get("Network")?.as_object()?;

    let looks_binned = net.keys().any(|k| k.starts_with("CpueQueue"));

    let mut cpus: Vec<(Option<u32>, String, CpuAffinitySiteTreeNode)> = Vec::new();

    if looks_binned {
        for (cpu_key, cpu_value) in net.iter() {
            let Some(cpu_node) = cpu_value.as_object() else {
                continue;
            };
            let mut site_children: Vec<CpuAffinitySiteTreeNode> = Vec::new();
            if let Some(Value::Object(child_map)) = cpu_node.get("children") {
                for (child_name, child_value) in child_map.iter() {
                    if let Value::Object(child_node) = child_value
                        && is_network_node(child_node)
                    {
                        site_children.push(build_site_tree(child_name, child_node));
                    }
                }
            }
            site_children.sort_by(|a, b| a.name.cmp(&b.name));

            let cpu_idx = trailing_u32(cpu_key);
            let cpu_label = cpu_idx
                .map(|n| format!("CPU {n}"))
                .unwrap_or_else(|| cpu_key.to_string());

            cpus.push((
                cpu_idx,
                cpu_key.to_string(),
                CpuAffinitySiteTreeNode {
                    name: cpu_label,
                    children: site_children,
                },
            ));
        }
    } else {
        // Non-binned network: nodes are top-level and each contains cpuNum/classMajor.
        let mut by_cpu: HashMap<Option<u32>, Vec<CpuAffinitySiteTreeNode>> = HashMap::new();
        for (child_name, child_value) in net.iter() {
            let Some(child_node) = child_value.as_object() else {
                continue;
            };
            if !is_network_node(child_node) {
                continue;
            }
            let cpu_idx = child_node
                .get("cpuNum")
                .and_then(|v| v.as_str())
                .and_then(parse_hex_u32)
                .or_else(|| {
                    child_node
                        .get("classMajor")
                        .and_then(|v| v.as_str())
                        .and_then(parse_hex_u32)
                        .map(|v| v.saturating_sub(1))
                });
            by_cpu
                .entry(cpu_idx)
                .or_default()
                .push(build_site_tree(child_name, child_node));
        }

        for (cpu_idx, mut nodes) in by_cpu.into_iter() {
            nodes.sort_by(|a, b| a.name.cmp(&b.name));
            let cpu_label = cpu_idx
                .map(|n| format!("CPU {n}"))
                .unwrap_or_else(|| "Unknown CPU".to_string());
            cpus.push((
                cpu_idx,
                cpu_label.clone(),
                CpuAffinitySiteTreeNode {
                    name: cpu_label,
                    children: nodes,
                },
            ));
        }
    }

    cpus.sort_by(|a, b| match (a.0, b.0) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.1.cmp(&b.1),
    });

    Some(CpuAffinitySiteTreeNode {
        name: "CPUs".to_string(),
        children: cpus.into_iter().map(|(_, _, n)| n).collect(),
    })
}

/// Build the runtime-aware CPU affinity snapshot used by the CPU assignment page.
pub fn cpu_affinity_runtime_snapshot_data() -> CpuAffinityRuntimeSnapshot {
    let live_nodes = network_tree_lite_data();
    let planned_cpu_by_name = load_planned_node_cpu_map();
    let placements = derive_runtime_node_placements(&live_nodes, &planned_cpu_by_name);
    let direct_circuit_counts = direct_circuit_counts_by_node();
    let (subtree_node_counts, subtree_circuit_counts) =
        build_subtree_counts(&live_nodes, &direct_circuit_counts);
    let circuits = load_all_circuits();
    let planned_core_metrics = aggregate_planned_core_metrics(&circuits);
    let live_cpu_usage = live_cpu_usage_by_index();

    let mut node_index_by_name = HashMap::new();
    for (idx, (_, node)) in live_nodes.iter().enumerate() {
        node_index_by_name.insert(node.name.clone(), idx);
    }

    let mut cores_by_cpu: HashMap<u32, CpuAffinityRuntimeCore> = HashMap::new();
    let mut all_cpus = BTreeSet::new();

    for cpu in live_cpu_usage.keys().copied() {
        all_cpus.insert(cpu);
    }
    for cpu in planned_core_metrics.keys().copied() {
        all_cpus.insert(cpu);
    }

    for idx in 1..live_nodes.len() {
        let node = &live_nodes[idx].1;
        let Some(cpu) = placements[idx].effective_cpu else {
            continue;
        };
        all_cpus.insert(cpu);
        let core = cores_by_cpu
            .entry(cpu)
            .or_insert_with(|| CpuAffinityRuntimeCore {
                cpu,
                live_usage_pct: live_cpu_usage.get(&cpu).copied(),
                planned_circuit_count: 0,
                planned_weight_sum: 0.0,
                planned_max_mbps: 0.0,
                effective_node_count: 0,
                effective_site_count: 0,
                effective_ap_count: 0,
                effective_circuit_count: 0,
                runtime_changed_count: 0,
                nodes: Vec::new(),
            });

        core.effective_node_count = core.effective_node_count.saturating_add(1);
        if is_site_type(node.node_type.as_deref()) {
            core.effective_site_count = core.effective_site_count.saturating_add(1);
        }
        if is_ap_type(node.node_type.as_deref()) {
            core.effective_ap_count = core.effective_ap_count.saturating_add(1);
        }
        if placements[idx].assignment_reason != "planned"
            || placements[idx].planned_cpu != placements[idx].effective_cpu
        {
            core.runtime_changed_count = core.runtime_changed_count.saturating_add(1);
        }

        core.nodes.push(CpuAffinityRuntimeNode {
            name: node.name.clone(),
            node_type: node.node_type.clone(),
            planned_cpu: placements[idx].planned_cpu,
            effective_cpu: placements[idx].effective_cpu,
            runtime_virtualized: node.runtime_virtualized,
            assignment_reason: placements[idx].assignment_reason.to_string(),
            is_cpu_root: is_cpu_root_node(&live_nodes, &placements, idx),
            subtree_node_count: subtree_node_counts.get(idx).copied().unwrap_or_default(),
            subtree_circuit_count: subtree_circuit_counts.get(idx).copied().unwrap_or_default(),
            effective_max_mbps: node.max_throughput,
            current_throughput_bps: node.current_throughput,
        });
    }

    for (node_name, circuit_count) in direct_circuit_counts {
        let Some(node_idx) = node_index_by_name.get(node_name.as_str()).copied() else {
            continue;
        };
        let Some(cpu) = placements
            .get(node_idx)
            .and_then(|placement| placement.effective_cpu)
        else {
            continue;
        };
        all_cpus.insert(cpu);
        let core = cores_by_cpu
            .entry(cpu)
            .or_insert_with(|| CpuAffinityRuntimeCore {
                cpu,
                live_usage_pct: live_cpu_usage.get(&cpu).copied(),
                planned_circuit_count: 0,
                planned_weight_sum: 0.0,
                planned_max_mbps: 0.0,
                effective_node_count: 0,
                effective_site_count: 0,
                effective_ap_count: 0,
                effective_circuit_count: 0,
                runtime_changed_count: 0,
                nodes: Vec::new(),
            });
        core.effective_circuit_count = core.effective_circuit_count.saturating_add(circuit_count);
    }

    let generated_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);

    let shaping_detection = load_config()
        .ok()
        .map(|cfg| detect_shaping_cpus(cfg.as_ref()));
    let (shaping_cpus, excluded_cpus, has_hybrid_split) = resolve_snapshot_cpu_sets(
        shaping_detection,
        &all_cpus,
        &planned_core_metrics,
        &placements,
    );

    let mut cores = Vec::with_capacity(all_cpus.len());
    for cpu in all_cpus {
        let planned = planned_core_metrics.get(&cpu).copied().unwrap_or_default();
        let mut core = cores_by_cpu
            .remove(&cpu)
            .unwrap_or_else(|| CpuAffinityRuntimeCore {
                cpu,
                live_usage_pct: live_cpu_usage.get(&cpu).copied(),
                planned_circuit_count: 0,
                planned_weight_sum: 0.0,
                planned_max_mbps: 0.0,
                effective_node_count: 0,
                effective_site_count: 0,
                effective_ap_count: 0,
                effective_circuit_count: 0,
                runtime_changed_count: 0,
                nodes: Vec::new(),
            });
        core.live_usage_pct = live_cpu_usage.get(&cpu).copied().or(core.live_usage_pct);
        core.planned_circuit_count = planned.circuit_count;
        core.planned_weight_sum = planned.weight_sum;
        core.planned_max_mbps = planned.max_mbps;
        core.nodes.sort_by(|left, right| {
            let left_tp = left.current_throughput_bps.0 + left.current_throughput_bps.1;
            let right_tp = right.current_throughput_bps.0 + right.current_throughput_bps.1;
            right_tp
                .cmp(&left_tp)
                .then_with(|| left.name.cmp(&right.name))
        });
        cores.push(core);
    }

    CpuAffinityRuntimeSnapshot {
        generated_at_unix_ms,
        shaping_cpus,
        excluded_cpus,
        has_hybrid_split,
        cores,
    }
}

pub fn cpu_affinity_summary_data() -> Vec<CpuAffinitySummaryEntry> {
    let circuits = load_all_circuits();
    let mut down: HashMap<u32, (usize, f64, f64, f64)> = HashMap::new();
    let mut up: HashMap<u32, (usize, f64, f64, f64)> = HashMap::new();

    for c in circuits.iter() {
        let ignored = c.has_planner_weight && c.planner_weight <= 0.0;
        if let Some(cpu) = c.cpu_down
            && !ignored
        {
            let entry = down.entry(cpu).or_insert((0, 0.0, 0.0, 0.0));
            entry.0 += 1;
            entry.1 += c.min_down;
            entry.2 += c.max_down;
            entry.3 += c.planner_weight;
        }
        if let Some(cpu) = c.cpu_up
            && !ignored
        {
            let entry = up.entry(cpu).or_insert((0, 0.0, 0.0, 0.0));
            entry.0 += 1;
            entry.1 += c.min_up;
            entry.2 += c.max_up;
            entry.3 += c.planner_weight;
        }
    }

    // Union of CPUs seen in either direction
    let mut cpus: Vec<u32> = down.keys().chain(up.keys()).copied().collect();
    cpus.sort_unstable();
    cpus.dedup();

    let mut entries = Vec::with_capacity(cpus.len());
    for cpu in cpus.into_iter() {
        let d = down.get(&cpu).cloned().unwrap_or((0, 0.0, 0.0, 0.0));
        let u = up.get(&cpu).cloned().unwrap_or((0, 0.0, 0.0, 0.0));
        entries.push(CpuAffinitySummaryEntry {
            cpu,
            download: CpuSideSummary {
                circuits: d.0,
                min_sum_mbps: d.1,
                max_sum_mbps: d.2,
                weight_sum: d.3,
            },
            upload: CpuSideSummary {
                circuits: u.0,
                min_sum_mbps: u.1,
                max_sum_mbps: u.2,
                weight_sum: u.3,
            },
        });
    }

    entries
}

pub fn cpu_affinity_circuits_data(cpu: u32, q: CircuitsQuery) -> CpuAffinityCircuitsPage {
    let direction = q
        .direction
        .as_deref()
        .map(|s| {
            if s.eq_ignore_ascii_case("up") {
                "up"
            } else {
                "down"
            }
        })
        .unwrap_or("down");

    let page = q.page.unwrap_or(1).max(1);
    let page_size = q.page_size.unwrap_or(100).clamp(1, 1000);
    let search = q.search.as_deref().map(|s| s.to_lowercase());

    let circuits = load_all_circuits();
    let mut filtered: Vec<CircuitBrief> = circuits
        .into_iter()
        .filter(|c| match direction {
            "up" => c.cpu_up == Some(cpu),
            _ => c.cpu_down == Some(cpu),
        })
        .filter(|c| {
            if let Some(ref needle) = search {
                let id = c.circuit_id.as_deref().unwrap_or("").to_lowercase();
                let nm = c.circuit_name.as_deref().unwrap_or("").to_lowercase();
                id.contains(needle) || nm.contains(needle)
            } else {
                true
            }
        })
        .map(|c| CircuitBrief {
            circuit_id: c.circuit_id,
            circuit_name: c.circuit_name,
            parent_node: c.parent_node,
            classid: if direction == "up" {
                c.classid_up
            } else {
                c.classid_down
            },
            max_mbps: if direction == "up" {
                c.max_up
            } else {
                c.max_down
            },
            weight: c.planner_weight,
            ip_count: c.ip_count,
            ignored: c.has_planner_weight && c.planner_weight <= 0.0,
        })
        .collect();

    // Sort by circuit name, then id
    filtered.sort_by(|a, b| {
        let an = a.circuit_name.as_deref().unwrap_or("");
        let bn = b.circuit_name.as_deref().unwrap_or("");
        let ord = an.cmp(bn);
        if ord == std::cmp::Ordering::Equal {
            a.circuit_id
                .as_deref()
                .unwrap_or("")
                .cmp(b.circuit_id.as_deref().unwrap_or(""))
        } else {
            ord
        }
    });

    let total = filtered.len();
    let start = (page - 1) * page_size;
    let end = std::cmp::min(start + page_size, total);
    let page_items = if start < total {
        filtered[start..end].to_vec()
    } else {
        Vec::new()
    };

    CpuAffinityCircuitsPage {
        cpu,
        direction: direction.to_string(),
        total,
        page,
        page_size,
        items: page_items,
    }
}

// Return all circuits (direction-filtered) without pagination to support client-side previews.
pub fn cpu_affinity_circuits_all_data(q: CircuitsQuery) -> Vec<CircuitBrief> {
    let direction = q
        .direction
        .as_deref()
        .map(|s| {
            if s.eq_ignore_ascii_case("up") {
                "up"
            } else {
                "down"
            }
        })
        .unwrap_or("down");
    let search = q.search.as_deref().map(|s| s.to_lowercase());

    let circuits = load_all_circuits();
    let mut items: Vec<CircuitBrief> = circuits
        .into_iter()
        .filter(|c| {
            if let Some(ref needle) = search {
                let id = c.circuit_id.as_deref().unwrap_or("").to_lowercase();
                let nm = c.circuit_name.as_deref().unwrap_or("").to_lowercase();
                id.contains(needle) || nm.contains(needle)
            } else {
                true
            }
        })
        .map(|c| CircuitBrief {
            circuit_id: c.circuit_id,
            circuit_name: c.circuit_name,
            parent_node: c.parent_node,
            classid: if direction == "up" {
                c.classid_up
            } else {
                c.classid_down
            },
            max_mbps: if direction == "up" {
                c.max_up
            } else {
                c.max_down
            },
            weight: c.planner_weight,
            ip_count: c.ip_count,
            ignored: c.has_planner_weight && c.planner_weight <= 0.0,
        })
        .collect();

    // Sort deterministically
    items.sort_by(|a, b| {
        let an = a.circuit_name.as_deref().unwrap_or("");
        let bn = b.circuit_name.as_deref().unwrap_or("");
        let ord = an.cmp(bn);
        if ord == std::cmp::Ordering::Equal {
            a.circuit_id
                .as_deref()
                .unwrap_or("")
                .cmp(b.circuit_id.as_deref().unwrap_or(""))
        } else {
            ord
        }
    });

    items
}

#[derive(Serialize, Clone, Debug)]
pub struct PreviewWeightItem {
    pub key: String,
    pub weight: f64,
}

// Minimal payload for preview: just (key, weight) for each circuit in a direction.
pub fn cpu_affinity_preview_weights_data(q: CircuitsQuery) -> Vec<PreviewWeightItem> {
    let direction = q
        .direction
        .as_deref()
        .map(|s| {
            if s.eq_ignore_ascii_case("up") {
                "up"
            } else {
                "down"
            }
        })
        .unwrap_or("down");
    let search = q.search.as_deref().map(|s| s.to_lowercase());

    let circuits = load_all_circuits();
    let mut items: Vec<PreviewWeightItem> = circuits
        .into_iter()
        .filter(|c| {
            if let Some(ref needle) = search {
                let id = c.circuit_id.as_deref().unwrap_or("").to_lowercase();
                let nm = c.circuit_name.as_deref().unwrap_or("").to_lowercase();
                id.contains(needle) || nm.contains(needle)
            } else {
                true
            }
        })
        .map(|c| PreviewWeightItem {
            key: c
                .circuit_id
                .clone()
                .or(c.circuit_name.clone())
                .unwrap_or_default(),
            weight: if direction == "up" {
                c.max_up
            } else {
                c.max_down
            },
        })
        .collect();

    // Deterministic ordering for ties
    items.sort_by(|a, b| {
        let ow = b
            .weight
            .partial_cmp(&a.weight)
            .unwrap_or(std::cmp::Ordering::Equal);
        if ow == std::cmp::Ordering::Equal {
            a.key.cmp(&b.key)
        } else {
            ow
        }
    });

    items
}

#[cfg(test)]
mod tests {
    use super::{
        PlannedCoreMetrics, RuntimeNodePlacement, derive_runtime_node_placements, is_cpu_root_node,
        resolve_snapshot_cpu_sets,
    };
    use crate::node_manager::local_api::network_tree_lite::NetworkTreeLiteNode;
    use lqos_config::{ShapingCpuDetection, ShapingCpuSource};
    use std::collections::BTreeSet;
    use std::collections::HashMap;

    fn test_node(
        name: &str,
        immediate_parent: Option<usize>,
        runtime_virtualized: bool,
    ) -> NetworkTreeLiteNode {
        NetworkTreeLiteNode {
            name: name.to_string(),
            id: None,
            is_virtual: false,
            runtime_virtualized,
            max_throughput: (0.0, 0.0),
            current_throughput: (0, 0),
            current_tcp_packets: (0, 0),
            current_tcp_retransmit_packets: (0, 0),
            current_retransmits: (0, 0),
            rtts: Vec::new(),
            qoo: (None, None),
            parents: Vec::new(),
            immediate_parent,
            node_type: None,
            latitude: None,
            longitude: None,
        }
    }

    #[test]
    fn runtime_placements_collapse_virtualized_branch_into_parent_cpu() {
        let nodes = vec![
            (0, test_node("Root", None, false)),
            (1, test_node("Site A", Some(0), false)),
            (2, test_node("Site B", Some(1), true)),
            (3, test_node("Site C", Some(2), false)),
        ];
        let planned = HashMap::from([
            ("Site A".to_string(), 2u32),
            ("Site B".to_string(), 2u32),
            ("Site C".to_string(), 2u32),
        ]);

        let placements = derive_runtime_node_placements(&nodes, &planned);

        assert_eq!(
            placements[1],
            RuntimeNodePlacement {
                owner_index: Some(1),
                planned_cpu: Some(2),
                effective_cpu: Some(2),
                assignment_reason: "planned",
            }
        );
        assert_eq!(
            placements[2],
            RuntimeNodePlacement {
                owner_index: Some(1),
                planned_cpu: Some(2),
                effective_cpu: Some(2),
                assignment_reason: "runtime_virtualized_hidden",
            }
        );
        assert_eq!(
            placements[3],
            RuntimeNodePlacement {
                owner_index: Some(1),
                planned_cpu: Some(2),
                effective_cpu: Some(2),
                assignment_reason: "inherited_from_virtualized_ancestor",
            }
        );
    }

    #[test]
    fn runtime_placements_fall_back_to_top_level_virtualized_cpu() {
        let nodes = vec![
            (0, test_node("Root", None, false)),
            (1, test_node("Top Level", Some(0), true)),
        ];
        let planned = HashMap::from([("Top Level".to_string(), 5u32)]);

        let placements = derive_runtime_node_placements(&nodes, &planned);

        assert_eq!(
            placements[1],
            RuntimeNodePlacement {
                owner_index: Some(0),
                planned_cpu: Some(5),
                effective_cpu: Some(5),
                assignment_reason: "runtime_virtualized_hidden",
            }
        );
    }

    #[test]
    fn cpu_root_selection_keeps_only_top_level_owned_branch() {
        let nodes = vec![
            (0, test_node("Root", None, false)),
            (1, test_node("Site A", Some(0), false)),
            (2, test_node("AP A1", Some(1), false)),
            (3, test_node("AP A2", Some(2), false)),
        ];
        let planned = HashMap::from([
            ("Site A".to_string(), 2u32),
            ("AP A1".to_string(), 2u32),
            ("AP A2".to_string(), 2u32),
        ]);

        let placements = derive_runtime_node_placements(&nodes, &planned);

        assert!(is_cpu_root_node(&nodes, &placements, 1));
        assert!(!is_cpu_root_node(&nodes, &placements, 2));
        assert!(!is_cpu_root_node(&nodes, &placements, 3));
    }

    #[test]
    fn cpu_root_selection_ignores_virtualized_hidden_nodes() {
        let nodes = vec![
            (0, test_node("Root", None, false)),
            (1, test_node("Top Level", Some(0), true)),
            (2, test_node("Child", Some(1), false)),
        ];
        let planned = HashMap::from([("Top Level".to_string(), 5u32), ("Child".to_string(), 5u32)]);

        let placements = derive_runtime_node_placements(&nodes, &planned);

        assert!(!is_cpu_root_node(&nodes, &placements, 1));
        assert!(is_cpu_root_node(&nodes, &placements, 2));
    }

    #[test]
    fn snapshot_cpu_sets_fall_back_to_planned_subset_when_hybrid_split_missing() {
        let all_cpus = BTreeSet::from([0_u32, 1, 2, 3]);
        let planned_core_metrics = HashMap::from([
            (
                0_u32,
                PlannedCoreMetrics {
                    circuit_count: 12,
                    weight_sum: 1.0,
                    max_mbps: 1000.0,
                },
            ),
            (
                1_u32,
                PlannedCoreMetrics {
                    circuit_count: 9,
                    weight_sum: 1.0,
                    max_mbps: 800.0,
                },
            ),
        ]);
        let placements = vec![
            RuntimeNodePlacement {
                owner_index: Some(0),
                planned_cpu: Some(0),
                effective_cpu: Some(0),
                assignment_reason: "planned",
            },
            RuntimeNodePlacement {
                owner_index: Some(1),
                planned_cpu: Some(1),
                effective_cpu: Some(1),
                assignment_reason: "planned",
            },
        ];
        let detection = ShapingCpuDetection {
            exclude_efficiency_cores: true,
            source: ShapingCpuSource::FallbackAllPossible,
            from_cache: false,
            has_hybrid_split: false,
            detail: "No trustworthy hybrid CPU split detected; using all possible CPUs".to_string(),
            possible: vec![0, 1, 2, 3],
            performance: vec![0, 1, 2, 3],
            efficiency: Vec::new(),
            shaping: vec![0, 1, 2, 3],
        };

        let (shaping, excluded, has_hybrid_split) = resolve_snapshot_cpu_sets(
            Some(detection),
            &all_cpus,
            &planned_core_metrics,
            &placements,
        );

        assert_eq!(shaping, vec![0, 1]);
        assert_eq!(excluded, vec![2, 3]);
        assert!(!has_hybrid_split);
    }
}
