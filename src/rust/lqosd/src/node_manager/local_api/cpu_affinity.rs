use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

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
            if let Some((maj, _)) = s.split_once(':') {
                if let Some(v) = parse_hex_u32(maj) {
                    rec.cpu_down = Some(v.saturating_sub(1));
                }
            }
        }
        if let Some(Value::String(s)) = cm.get("up_cpuNum") {
            rec.cpu_up = parse_hex_u32(s);
        } else if let Some(Value::String(s)) = cm.get("up_classMajor") {
            if let Some(v) = parse_hex_u32(s) {
                rec.cpu_up = Some(v.saturating_sub(1));
            }
        } else if let Some(Value::String(s)) = cm.get("up_classid") {
            if let Some((maj, _)) = s.split_once(':') {
                if let Some(v) = parse_hex_u32(maj) {
                    rec.cpu_up = Some(v.saturating_sub(1));
                }
            }
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
        if let Some(Value::String(s)) = cm.get("circuitID") {
            if !s.is_empty() {
                rec.circuit_id = Some(s.clone());
            }
        }
        if let Some(Value::String(s)) = cm.get("circuitName") {
            if !s.is_empty() {
                rec.circuit_name = Some(s.clone());
            }
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
    if let Value::Object(map) = json {
        if let Some(Value::Object(net)) = map.get("Network") {
            for (_k, v) in net.iter() {
                if let Value::Object(node) = v {
                    add_circuit_records(None, node, &mut circuits);
                }
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
            if let Value::Object(child_node) = child_value {
                if is_network_node(child_node) {
                    children.push(build_site_tree(child_name, child_node));
                }
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
                    if let Value::Object(child_node) = child_value {
                        if is_network_node(child_node) {
                            site_children.push(build_site_tree(child_name, child_node));
                        }
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

pub fn cpu_affinity_summary_data() -> Vec<CpuAffinitySummaryEntry> {
    let circuits = load_all_circuits();
    let mut down: HashMap<u32, (usize, f64, f64, f64)> = HashMap::new();
    let mut up: HashMap<u32, (usize, f64, f64, f64)> = HashMap::new();

    for c in circuits.iter() {
        let ignored = c.has_planner_weight && c.planner_weight <= 0.0;
        if let Some(cpu) = c.cpu_down {
            if !ignored {
                let entry = down.entry(cpu).or_insert((0, 0.0, 0.0, 0.0));
                entry.0 += 1;
                entry.1 += c.min_down;
                entry.2 += c.max_down;
                entry.3 += c.planner_weight;
            }
        }
        if let Some(cpu) = c.cpu_up {
            if !ignored {
                let entry = up.entry(cpu).or_insert((0, 0.0, 0.0, 0.0));
                entry.0 += 1;
                entry.1 += c.min_up;
                entry.2 += c.max_up;
                entry.3 += c.planner_weight;
            }
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
