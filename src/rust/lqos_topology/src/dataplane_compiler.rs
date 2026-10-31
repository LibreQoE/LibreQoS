//! Product-side transaction COMPILER: effective network + shaping inputs
//! → a [`DataplaneTransaction`] (schema lives in lqos_dataplane_types — the wire contract).

use lqos_config::{DynamicCircuitRangeRule, TopologyShapingCircuitInput, TopologyShapingInputsFile};
use lqos_dataplane_types::*;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;

use crate::read_node_rate_mbps;

/// `lqos-74t.3` (DRIVER: `orphans` ONLY): the synthetic no-path umbrella's
/// canonical node id — ALWAYS emitted (never elided when childless) so it does
/// not toggle create/remove. ONE HOME (Fix A): aliases the shared
/// `lqos_dataplane_types` constant so the producer and the guard's compiler can
/// never drift on the literal.
// `lqos-74t.15.2` (A1): the former `ORPHANS_UMBRELLA_NODE_ID` local alias is
// RETIRED (the `orphans`-only special-case is subsumed by the GENERAL childless
// keep). The ONE-HOME id stays in `lqos_dataplane_types::ORPHANS_UMBRELLA_NODE_ID`.


/// Compiles a full desired-state transaction from the effective network tree and
/// shaping inputs.
///
/// Emits an [`DataplaneInnerOp::Upsert`] for every CIRCUIT-BEARING node (top-level
/// nodes hang off [`DATA_PLANE_ROOT_NODE_ID`]), a [`DataplaneLeafOp::Upsert`] for
/// every circuit, and a [`DataplaneMappingOp::Map`] for every device IP. Does not
/// diff; callers wanting an incremental transaction run the diff layer on top.
///
/// F-LM (lkg_boot_mirror_card §9): the transaction is the SHAPING contract — a
/// circuit-less site is shaping-inert. Nodes whose subtree bears no circuit are
/// ELIDED here (one translation point); the bridge's N1 scope gate stays verbatim
/// and a circuit add under a previously-elided site re-materializes the full
/// chain in one add-carrying payload.
pub fn compile_dataplane_transaction(
    effective_network: &Value,
    shaping_inputs: &TopologyShapingInputsFile,
) -> DataplaneTransaction {
    compile_dataplane_transaction_with_ranges_and_sqm(effective_network, shaping_inputs, &[], "none")
}

/// Dynamic-ranges H1: compiles the transaction WITH the configured pool ranges
/// injected at the TRANSACTION stage (post-catalog). Each range emits the
/// unconfirmed leaf + its prefix Map. These are NEVER shaping-input rows, so
/// the K1 promotion guard, supersede-prune, analytics preference, and TTL
/// liveness all keep behaving (a static row would break all four).
pub fn compile_dataplane_transaction_with_ranges(
    effective_network: &Value,
    shaping_inputs: &TopologyShapingInputsFile,
    ranges: &[DynamicCircuitRangeRule],
) -> DataplaneTransaction {
    compile_dataplane_transaction_with_ranges_and_sqm(effective_network, shaping_inputs, ranges, "none")
}

/// Compiles the dataplane transaction honoring the box's configured
/// `[queues] default_sqm` for every circuit WITHOUT a per-circuit override
/// — the v2→v3 migration must not drop the SQM layer). The
/// production caller (`publish.rs`) passes `config.queues.default_sqm`.
pub fn compile_dataplane_transaction_with_ranges_and_sqm(
    effective_network: &Value,
    shaping_inputs: &TopologyShapingInputsFile,
    ranges: &[DynamicCircuitRangeRule],
    default_sqm: &str,
) -> DataplaneTransaction {
    let range_rows = resolve_range_rows(effective_network, ranges);
    let range_parent_ids: Vec<String> = range_rows
        .iter()
        .map(|row| row.parent_node_id.clone())
        .collect();
    let keep = circuit_bearing_nodes(effective_network, shaping_inputs, &range_parent_ids);
    let mut inner_ops = Vec::new();
    if let Some(root) = effective_network.as_object() {
        for node in root.values() {
            collect_inner_ops(node, DATA_PLANE_ROOT_NODE_ID, &mut inner_ops, &keep);
        }
    }
 // 3b rate fix(i)): the leaf clamp needs the effective parent
    // ceils — one pass over the tree.
    let node_rates = effective_node_rates(effective_network);

    let mut leaf_ops = Vec::new();
    let mut mapping_ops = Vec::new();
    for circuit in &shaping_inputs.circuits {
        leaf_ops.push(leaf_upsert(circuit, &node_rates, default_sqm));
        for device in &circuit.devices {
            for cidr in device.ipv4.iter().chain(device.ipv6.iter()) {
                if let Some((prefix_len, ip)) = parse_cidr(cidr) {
                    mapping_ops.push(DataplaneMappingOp::Map {
                        prefix_len,
                        ip,
                        circuit_id: circuit.circuit_id.clone(),
                        device_id: device.device_id.clone(),
                    });
                }
            }
        }
        // T-arc §2.8: a circuit carrying a tunnel identity also emits a
        // MapTunnel (the (VNI, VLAN) classification key), alongside its IP maps.
        if let Some(identity) = circuit.tunnel_identity {
            mapping_ops.push(DataplaneMappingOp::MapTunnel {
                vni: identity.vni,
                vlan: identity.vlan,
                circuit: circuit.circuit_id.clone(),
            });
        }
        // Phase C S2: the vlan / mpls identities compile to their exact-match
        // ops (the modes are mutually exclusive per circuit; the classifier
        // fronts key on them).
        if let Some(vlan) = circuit.vlan_identity {
            mapping_ops.push(DataplaneMappingOp::MapVlan {
                vlan,
                circuit: circuit.circuit_id.clone(),
            });
        }
        if let Some(label) = circuit.mpls_identity {
            mapping_ops.push(DataplaneMappingOp::MapMpls {
                label,
                circuit: circuit.circuit_id.clone(),
            });
        }
    }

    // Dynamic-ranges H1: the unconfirmed leaf + prefix Map per range. The
    // prefix Map is a longest-prefix entry, so a promoted /32 shadows it.
    for row in &range_rows {
        leaf_ops.push(range_leaf(row, &node_rates));
        mapping_ops.push(DataplaneMappingOp::Map {
            prefix_len: row.prefix_len,
            ip: row.network_address,
            circuit_id: row.circuit_id.clone(),
            device_id: row.circuit_id.clone(),
        });
    }

    DataplaneTransaction {
        generation: shaping_inputs.shaping_generation.clone(),
        inner_ops,
        leaf_ops,
        mapping_ops,
        // M1 (config card) via the wire const (protocol-v2 closure 3):
        // the compiler EMITS the version — one source.
        schema_version: Some(lqos_dataplane_types::wire::PROTOCOL_VERSION),
    
        steering_vlans: Vec::new(), lpm_delta: Vec::new(), apply_mode: Default::default(), circuit_capacity: None, }
}

/// One resolved range row (config → transaction-stage leaf + Map).
struct RangeRow {
    circuit_id: String,
    parent_node_id: String,
    prefix_len: u8,
    network_address: IpAddr,
    download_min_mbps: f32,
    download_max_mbps: f32,
    upload_min_mbps: f32,
    upload_max_mbps: f32,
}

/// Resolves the configured ranges against the effective tree. §1.5: an
/// unresolvable parent DEFERS the whole range row LOUDLY (no leaf, no Map).
fn resolve_range_rows(
    effective_network: &Value,
    ranges: &[DynamicCircuitRangeRule],
) -> Vec<RangeRow> {
    if ranges.is_empty() {
        return Vec::new();
    }
    let name_to_id = effective_node_name_to_id(effective_network);
    let mut rows = Vec::new();
    for rule in ranges {
        let range_id = rule.name.trim();
        let attach = rule.attach_to.trim();
        // Blank/whitespace attach_to is the dataplane root: the root node is
        // implicit (never emitted as an inner op) and unclamped, so the range
        // leaf needs no parent-rate lookup and must not be deferred.
        let parent_node_id = if attach.is_empty() {
            DATA_PLANE_ROOT_NODE_ID.to_string()
        } else {
            let Some(parent_node_id) = name_to_id.get(attach).cloned() else {
                tracing::warn!(
                    "dynamic_range_parent_unresolved: range '{range_id}' attach_to {attach:?} matches no effective node — the range is DEFERRED (no unconfirmed leaf, no prefix Map)"
                );
                continue;
            };
            parent_node_id
        };
        rows.push(RangeRow {
            circuit_id: format!("dynamic-range:{range_id}"),
            parent_node_id,
            prefix_len: rule.ip_range.netmask(),
            network_address: rule.ip_range.network_address(),
            download_min_mbps: rule.download_min_mbps,
            download_max_mbps: rule.download_max_mbps,
            upload_min_mbps: rule.upload_min_mbps,
            upload_max_mbps: rule.upload_max_mbps,
        });
    }
    rows
}

/// Maps each effective-tree node's display name (the `name` field, else the
/// object key — the fold's fallback) to its node id.
fn effective_node_name_to_id(root: &Value) -> HashMap<String, String> {
    fn walk(name: &str, node: &Value, out: &mut HashMap<String, String>) {
        let Some(obj) = node.as_object() else {
            return;
        };
        let Some(id) = obj.get("id").and_then(Value::as_str) else {
            return;
        };
        let display = obj.get("name").and_then(Value::as_str).unwrap_or(name);
        out.insert(display.to_string(), id.to_string());
        if let Some(children) = obj.get("children").and_then(Value::as_object) {
            for (child_name, child) in children {
                walk(child_name, child, out);
            }
        }
    }
    let mut out = HashMap::new();
    if let Some(root) = root.as_object() {
        for (name, node) in root {
            walk(name, node, &mut out);
        }
    }
    out
}

fn range_leaf(
    row: &RangeRow,
    node_rates: &HashMap<String, (DataplaneRatePair, DataplaneRatePair)>,
) -> DataplaneLeafOp {
    let parent = node_rates.get(&row.parent_node_id);
    DataplaneLeafOp::Upsert {
        circuit_id: row.circuit_id.clone(),
        parent_node_id: row.parent_node_id.clone(),
        download: clamp_leaf_to_parent(
            rate_pair(row.download_min_mbps, row.download_max_mbps),
            parent.map(|pair| pair.0),
        ),
        upload: clamp_leaf_to_parent(
            rate_pair(row.upload_min_mbps, row.upload_max_mbps),
            parent.map(|pair| pair.1),
        ),
        // The unconfirmed trickle class carries no AQM (the F5 bounded floor).
        aqm: DataplaneLeafAqm::None,
        admin_up: true,
    }
}

/// §`lqos-74t.30` (encode-once): the LEAF rate pair. The whole-Mbps product
/// rounding zeroes the suspend sentinel (`0.01`) **AND any sub-0.5 Mbps config**,
/// and a `0/0` pair is `is_disabled()` == FULL line rate at the bridge -- so the
/// PAIR-LEVEL floor (`lqos_dataplane_types::floor_suspended_pair`, the shared
/// home) maps it to `SUSPEND_RATE_KBPS`. Only LEAF rates pass through here;
/// inner nodes use `node_rate_pair` (never the suspend sentinel). The guard
/// compiler applies the SAME helper, so the two emissions cannot drift
/// (`assert_analysis_emission_equality`).
fn rate_pair(min_mbps: f32, max_mbps: f32) -> DataplaneRatePair {
    let mut pair = DataplaneRatePair {
        commit_kbps: (min_mbps.round() as u64 * 1000).min(u64::from(u32::MAX)) as u32,
        ceil_kbps: (max_mbps.round() as u64 * 1000).min(u64::from(u32::MAX)) as u32,
    };
    floor_suspended_pair(&mut pair);
    pair
}

/// Maps every effective-tree node id to its `(download, upload)` rate pair
/// (kbps). Used by the leaf clamp; the inner emitter reads the nodes directly.
fn effective_node_rates(
    root: &Value,
) -> HashMap<String, (DataplaneRatePair, DataplaneRatePair)> {
    fn walk(
        node: &Value,
        out: &mut HashMap<String, (DataplaneRatePair, DataplaneRatePair)>,
    ) {
        let Some(obj) = node.as_object() else {
            return;
        };
        let Some(node_id) = obj.get("id").and_then(Value::as_str) else {
            return;
        };
        out.insert(
            node_id.to_string(),
            (node_rate_pair(obj, "download"), node_rate_pair(obj, "upload")),
        );
        if let Some(children) = obj.get("children").and_then(Value::as_object) {
            for child in children.values() {
                walk(child, out);
            }
        }
    }
    let mut out = HashMap::new();
    if let Some(root) = root.as_object() {
        for node in root.values() {
            walk(node, &mut out);
        }
    }
    out
}

/// 3b rate fix(i)): ports the bridge's `expected_manifest` leaf clamp
/// into the transaction — `ceil = min(own ceil, parent effective ceil)` and
/// `commit = min(own commit, clamped ceil)`. The bridge's placement validation
/// then passes BY CONSTRUCTION, and malformed upstream data (e.g. a client
/// inheriting a whole-network PN sentinel) can never produce an inconsistent
/// transaction. A missing parent (orphan) leaves the leaf unclamped — matching
/// the bridge's independently derived manifest. PURE.
fn clamp_leaf_to_parent(
    own: DataplaneRatePair,
    parent: Option<DataplaneRatePair>,
) -> DataplaneRatePair {
    let ceil_kbps = match parent {
        Some(parent) => own.ceil_kbps.min(parent.ceil_kbps),
        None => own.ceil_kbps,
    };
    DataplaneRatePair {
        commit_kbps: own.commit_kbps.min(ceil_kbps),
        ceil_kbps,
    }
}

/// The F-LM KEEP set: every node id lying on ≥1 circuit's root→parent
/// ancestry chain (SUBTREE-scoped rule — keep iff ≥1 circuit's subtree
/// ancestry passes through the node). Any node not in the set has a
/// circuit-less subtree by construction (a circuit through a descendant
/// would keep this node), so pruning it prunes exactly its inert chain.
fn circuit_bearing_nodes(
    effective_network: &Value,
    shaping_inputs: &TopologyShapingInputsFile,
    range_parent_ids: &[String],
) -> HashSet<String> {
    fn collect_parents(
        node: &Value,
        parent: Option<&str>,
        parents: &mut Vec<(String, String)>,
        all_ids: &mut Vec<String>,
        has_children: &mut HashSet<String>,
    ) {
        let Some(obj) = node.as_object() else {
            return;
        };
        let Some(id) = obj.get("id").and_then(Value::as_str) else {
            return;
        };
        if let Some(parent) = parent {
            parents.push((id.to_string(), parent.to_string()));
        }
        all_ids.push(id.to_string());
        let children = obj.get("children").and_then(Value::as_object);
        if children.is_some_and(|children| !children.is_empty()) {
            has_children.insert(id.to_string());
        }
        if let Some(children) = children {
            for child in children.values() {
                collect_parents(child, Some(id), parents, all_ids, has_children);
            }
        }
    }
    let mut parent_pairs = Vec::new();
    let mut all_ids = Vec::new();
    let mut has_children = HashSet::new();
    if let Some(root) = effective_network.as_object() {
        for node in root.values() {
            collect_parents(node, None, &mut parent_pairs, &mut all_ids, &mut has_children);
        }
    }
    let parents: std::collections::HashMap<&str, &str> = parent_pairs
        .iter()
        .map(|(id, parent)| (id.as_str(), parent.as_str()))
        .collect();

    // The circuit half of the A1 unit predicate: node ids with ≥1 circuit
    // parented directly at them.
    let circuit_parent_ids: HashSet<&str> = shaping_inputs
        .circuits
        .iter()
        .map(|circuit| circuit.effective_parent_node_id.as_str())
        .collect();

    let mut keep = HashSet::new();
    for circuit in &shaping_inputs.circuits {
        keep_ancestry(circuit.effective_parent_node_id.as_str(), &parents, &mut keep);
    }
    // Dynamic-ranges H1: the range leaf's parent ancestry must materialize too
    // (a range under an otherwise circuit-less site would be elided).
    for parent in range_parent_ids {
        keep_ancestry(parent.as_str(), &parents, &mut keep);
    }
    // A1 GENERAL rule (the ONE-HOME predicate
    // `lqos_dataplane_types::is_childless_circuitless_unit`) — SUPERSEDING the
    // former `orphans`-only exemption, which it subsumes: EVERY childless,
    // circuit-less node is load-bearing (the analysis `Unit` rule) and is KEPT
    // with its ROOT ancestry. The guard compiler
    // (`transaction_compiler::compiler::circuit_bearing_nodes`) calls the SAME
    // helper, so both emit the same world or the boot
    // `assert_analysis_emission_equality` FATALs (`AnalysisEmissionDivergence`).
    for id in &all_ids {
        if lqos_dataplane_types::is_childless_circuitless_unit(
            !has_children.contains(id),
            circuit_parent_ids.contains(id.as_str()),
        ) {
            keep_ancestry(id, &parents, &mut keep);
        }
    }
    keep
}

fn keep_ancestry(
    start: &str,
    parents: &std::collections::HashMap<&str, &str>,
    keep: &mut HashSet<String>,
) {
    let mut cursor = start;
    while !cursor.is_empty() && cursor != DATA_PLANE_ROOT_NODE_ID {
        // Already kept ⇒ its whole ancestry was kept by the walk
        // that inserted it — stop early.
        if !keep.insert(cursor.to_string()) {
            break;
        }
        match parents.get(cursor) {
            Some(next) => cursor = next,
            None => break,
        }
    }
}

/// Recursively emits inner-node Upserts for a node and its kept
/// descendants. A node outside `keep` ends the walk: its entire subtree
/// is circuit-less (the F-LM subtree-scoped elision).
fn collect_inner_ops(
    node: &Value,
    parent_id: &str,
    out: &mut Vec<DataplaneInnerOp>,
    keep: &HashSet<String>,
) {
    let Some(obj) = node.as_object() else {
        return;
    };
    let Some(node_id) = obj.get("id").and_then(Value::as_str) else {
        return;
    };
    if !keep.contains(node_id) {
        return;
    }
    out.push(DataplaneInnerOp::Upsert {
        node_id: node_id.to_string(),
        parent_node_id: parent_id.to_string(),
        download: node_rate_pair(obj, "download"),
        upload: node_rate_pair(obj, "upload"),
    });
    if let Some(children) = obj.get("children").and_then(Value::as_object) {
        for child in children.values() {
            collect_inner_ops(child, node_id, out, keep);
        }
    }
}

/// Reads a node's directional commit/ceil from `*BandwidthMbpsMin`/`*BandwidthMbps`.
///
/// `lqos-74t.15.2` follow-on: the arithmetic is the ONE-HOME shared derivation
/// (`lqos_dataplane_types::inner_node_rate_pair`) — the guard compiler calls the
/// SAME function, so the inner commit cannot drift across the seam (pre-code pin 3).
fn node_rate_pair(
    obj: &serde_json::Map<String, Value>,
    direction: &str,
) -> DataplaneRatePair {
    let min_mbps = read_node_rate_mbps(obj, &format!("{direction}BandwidthMbpsMin")).unwrap_or(0);
    let ceil_mbps = read_node_rate_mbps(obj, &format!("{direction}BandwidthMbps")).unwrap_or(0);
 // §8b ruling: kbps-native; the shared fn converts x1000 at emit,
    // exact + u32-safe (100G = 1e8 kbps).
    lqos_dataplane_types::inner_node_rate_pair(ceil_mbps, min_mbps)
}

/// Builds a leaf Upsert from one shaping-input circuit, clamped to its
/// effective parent's ceil (the bridge `expected_manifest` invariant).
fn leaf_upsert(
    circuit: &TopologyShapingCircuitInput,
    node_rates: &HashMap<String, (DataplaneRatePair, DataplaneRatePair)>,
    default_sqm: &str,
) -> DataplaneLeafOp {
    let parent = node_rates.get(&circuit.effective_parent_node_id);
    DataplaneLeafOp::Upsert {
        circuit_id: circuit.circuit_id.clone(),
        parent_node_id: circuit.effective_parent_node_id.clone(),
 // §8b ruling: Mbps inputs, kbps wire — x1000 at emit.
        download: clamp_leaf_to_parent(
            rate_pair(circuit.download_min_mbps, circuit.download_max_mbps),
            parent.map(|pair| pair.0),
        ),
        upload: clamp_leaf_to_parent(
            rate_pair(circuit.upload_min_mbps, circuit.upload_max_mbps),
            parent.map(|pair| pair.1),
        ),
        aqm: aqm_with_default(circuit.sqm_override.as_deref(), default_sqm),
        admin_up: true,
    }
}

/// Maps a raw `sqm_override` token to a leaf AQM policy.
///
/// This is the simplified form; the full `effective_sqm_str` port (down/up split,
/// RTT scaling) lands separately (finding #6).
pub fn aqm_from_override(override_token: Option<&str>) -> DataplaneLeafAqm {
    let token = override_token.map(str::trim).filter(|s| !s.is_empty());
    match token {
        None | Some("none") => DataplaneLeafAqm::None,
        Some(t) if t.starts_with("cake") => DataplaneLeafAqm::Cake {
            diffserv4: t.contains("diffserv4"),
            rtt_ms: t
                .split_whitespace()
                .find_map(|word| word.strip_prefix("rtt").and_then(|v| v.parse::<u32>().ok())),
        },
        Some(_) => DataplaneLeafAqm::None,
    }
}

/// The leaf AQM for an upgrade-safe world: a per-circuit
/// `sqm_override` wins when present, otherwise the box's configured
/// `[queues] default_sqm` applies — so a v2→v3 migration no longer silently
/// drops the SQM layer and runs every leaf FIFO. An explicit `none` override
/// (or a `none`/absent default) still means FIFO.
pub fn aqm_with_default(override_token: Option<&str>, default_sqm: &str) -> DataplaneLeafAqm {
    let explicit = override_token.map(str::trim).filter(|s| !s.is_empty());
    match explicit {
        Some(token) => aqm_from_override(Some(token)),
        None => aqm_from_override(Some(default_sqm)),
    }
}

/// Parses a `ip/prefix` CIDR string into `(prefix_len, ip)`. R0
/// (remove_lpm_completeness_card, fresh-#12: a BARE IP is a
/// host address (/32 v4, /128 v6) — the same normalization
/// lqos_network_devices/runtime_inputs.rs already applies (its
/// normalizers are why shaping_inputs carry bare IPs in the first
/// place). Before R0 the bare form fell through `split_once('/')?`
/// and the entry was dropped SILENTLY — the entire legacy pipeline
/// compiled zero maps (the mapless-universe root). Unparseable input
/// is skipped with a named warn — never silent again.
fn parse_cidr(cidr: &str) -> Option<(u8, IpAddr)> {
    let value = cidr.trim();
    let (ip_str, prefix_str) = match value.split_once('/') {
        Some((ip, prefix)) => (ip, Some(prefix)),
        None => (value, None),
    };
    let Ok(ip) = ip_str.trim().parse::<IpAddr>() else {
        tracing::warn!(
            "device_ip_unparseable: {cidr:?} is neither a CIDR nor an IP —              the device's map entry is SKIPPED and it will not classify"
        );
        return None;
    };
    let prefix = match prefix_str {
        Some(p) => match p.trim().parse::<u8>() {
            Ok(p) => p,
            Err(_) => {
                tracing::warn!(
                    "device_ip_unparseable: {cidr:?} has a bad prefix —                      the device's map entry is SKIPPED and it will not classify"
                );
                return None;
            }
        },
        None => match ip {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        },
    };
    Some((prefix, ip))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn effective_tree_with_parent_ceil(down_mbps: u64, up_mbps: u64) -> Value {
        json!({
            "ap-node": {
                "id": "uisp:device:af8cae80",
                "name": "AP",
                "downloadBandwidthMbps": down_mbps,
                "downloadBandwidthMbpsMin": down_mbps,
                "uploadBandwidthMbps": up_mbps,
                "uploadBandwidthMbpsMin": up_mbps,
            }
        })
    }

    fn circuit(parent: &str, min: f32, max: f32) -> TopologyShapingCircuitInput {
        TopologyShapingCircuitInput {
            circuit_id: "008b8b44".to_string(),
            circuit_name: "Carlos Marquez".to_string(),
            effective_parent_node_name: parent.to_string(),
            effective_parent_node_id: parent.to_string(),
            download_min_mbps: min,
            upload_min_mbps: min,
            download_max_mbps: max,
            upload_max_mbps: max,
            ..Default::default()
        }
    }

    fn leaf_rates(
        tx: &DataplaneTransaction,
        circuit_id: &str,
    ) -> (DataplaneRatePair, DataplaneRatePair) {
        tx.leaf_ops
            .iter()
            .find_map(|op| match op {
                DataplaneLeafOp::Upsert {
                    circuit_id: current,
                    download,
                    upload,
                    ..
                } if current == circuit_id => Some((download.clone(), upload.clone())),
                _ => None,
            })
            .expect("leaf present")
    }

 /// 3b rate fix(i), the.9 fixture): a 98G/100G client leaf under
    /// a 322 Mbps parent clamps to 322 Mbps (ceil AND commit) — the transaction
    /// now matches the bridge `expected_manifest` by construction.
    #[test]
    fn leaf_clamps_to_the_effective_parent_ceil() {
        let network = effective_tree_with_parent_ceil(322, 322);
        let inputs = TopologyShapingInputsFile {
            circuits: vec![circuit("uisp:device:af8cae80", 98_000.0, 100_000.0)],
            ..Default::default()
        };
        let tx = compile_dataplane_transaction(&network, &inputs);
        let (download, upload) = leaf_rates(&tx, "008b8b44");
        assert_eq!(download.ceil_kbps, 322_000, "ceil clamped to the parent");
        assert_eq!(
            download.commit_kbps, 322_000,
            "commit clamped under the clamped ceil"
        );
        assert_eq!(upload.ceil_kbps, 322_000);
        assert_eq!(upload.commit_kbps, 322_000);
    }

    /// A leaf at or below its parent's ceil is unchanged (no false clamping).
    #[test]
    fn leaf_below_the_parent_ceil_is_unchanged() {
        let network = effective_tree_with_parent_ceil(1_000, 500);
        let inputs = TopologyShapingInputsFile {
            circuits: vec![circuit("uisp:device:af8cae80", 200.0, 300.0)],
            ..Default::default()
        };
        let tx = compile_dataplane_transaction(&network, &inputs);
        let (download, upload) = leaf_rates(&tx, "008b8b44");
        assert_eq!(download.ceil_kbps, 300_000);
        assert_eq!(download.commit_kbps, 200_000);
        assert_eq!(upload.ceil_kbps, 300_000);
        assert_eq!(upload.commit_kbps, 200_000);
    }

    /// Root-attached circuits attach to an explicit synthetic site
    /// (`libreqos:root`), so the emitted leaf parent is never empty. The v3
    /// boot gate treats an empty parent as a defective transaction and holds
    /// forever; this is the compiler-side closure of that producer invariant.
    #[test]
    fn explicit_root_site_parent_is_emitted_never_empty() {
        let network = json!({
            "root-site": {
                "id": "libreqos:root",
                "name": "Unassigned / Root",
                "type": "site",
                "downloadBandwidthMbps": 1_000,
                "uploadBandwidthMbps": 1_000,
            }
        });
        let inputs = TopologyShapingInputsFile {
            circuits: vec![circuit("libreqos:root", 200.0, 300.0)],
            ..Default::default()
        };
        let tx = compile_dataplane_transaction(&network, &inputs);
        let leaf_parent = tx.leaf_ops.iter().find_map(|op| match op {
            DataplaneLeafOp::Upsert {
                circuit_id,
                parent_node_id,
                ..
            } if circuit_id == "008b8b44" => Some(parent_node_id.clone()),
            _ => None,
        });
        assert_eq!(leaf_parent.as_deref(), Some("libreqos:root"));
        assert!(
            leaf_parent.as_deref().is_some_and(|parent| !parent.trim().is_empty()),
            "root-attached leaf must carry an explicit parent"
        );
    }

    /// An orphan leaf (no parent node in the effective tree) is left unclamped —
    /// matching the bridge's independently derived manifest.
    #[test]
    fn orphan_leaf_is_not_clamped() {
        let network = json!({});
        let inputs = TopologyShapingInputsFile {
            circuits: vec![circuit("uisp:device:missing", 98_000.0, 100_000.0)],
            ..Default::default()
        };
        let tx = compile_dataplane_transaction(&network, &inputs);
        let (download, _upload) = leaf_rates(&tx, "008b8b44");
        assert_eq!(download.ceil_kbps, 100_000_000);
        assert_eq!(download.commit_kbps, 98_000_000);
    }

    /// §`lqos-74t.30` (encode-once): a suspended circuit (the `0.01` Mbps
    /// sentinel) and ANY sub-0.5 Mbps config would whole-Mbps-round to `0/0` ==
    /// `is_disabled()` == FULL line rate; the shared pair-level floor maps both
    /// to `SUSPEND_RATE_KBPS` (1024) instead.
    #[test]
    fn sub_half_mbps_leaf_floors_to_the_trickle_never_zero() {
        let network = json!({});
        // The suspend sentinel (min == max == 0.01 in both directions).
        let sentinel = compile_dataplane_transaction(
            &network,
            &TopologyShapingInputsFile {
                circuits: vec![circuit("uisp:device:missing", 0.01, 0.01)],
                ..Default::default()
            },
        );
        let (download, upload) = leaf_rates(&sentinel, "008b8b44");
        assert_eq!((download.commit_kbps, download.ceil_kbps), (1024, 1024));
        assert_eq!((upload.commit_kbps, upload.ceil_kbps), (1024, 1024));

        // A non-sentinel sub-0.5 config: still never 0/0 == full rate.
        let tiny = compile_dataplane_transaction(
            &network,
            &TopologyShapingInputsFile {
                circuits: vec![circuit("uisp:device:missing", 0.3, 0.3)],
                ..Default::default()
            },
        );
        let (download, _upload) = leaf_rates(&tiny, "008b8b44");
        assert_eq!((download.commit_kbps, download.ceil_kbps), (1024, 1024));
    }

    fn leaf_aqm(tx: &DataplaneTransaction, circuit_id: &str) -> DataplaneLeafAqm {
        tx.leaf_ops
            .iter()
            .find_map(|op| match op {
                DataplaneLeafOp::Upsert {
                    circuit_id: current,
                    aqm,
                    ..
                } if current == circuit_id => Some(*aqm),
                _ => None,
            })
            .expect("leaf present")
    }

 /// the box's `[queues] default_sqm` reaches every circuit
    /// WITHOUT a per-circuit override — the v2→v3 migration no longer drops
    /// the SQM layer (the fleet bufferbloat root cause).
    #[test]
    fn default_sqm_reaches_override_less_leaves() {
        let network = effective_tree_with_parent_ceil(1_000, 1_000);
        let inputs = TopologyShapingInputsFile {
            circuits: vec![circuit("uisp:device:af8cae80", 200.0, 300.0)],
            ..Default::default()
        };
        let tx = compile_dataplane_transaction_with_ranges_and_sqm(
            &network,
            &inputs,
            &[],
            "cake diffserv4",
        );
        assert_eq!(
            leaf_aqm(&tx, "008b8b44"),
            DataplaneLeafAqm::Cake {
                diffserv4: true,
                rtt_ms: None
            }
        );
    }

    /// An explicit per-circuit override wins over the box default.
    #[test]
    fn explicit_override_wins_over_default_sqm() {
        let network = effective_tree_with_parent_ceil(1_000, 1_000);
        let mut c = circuit("uisp:device:af8cae80", 200.0, 300.0);
        c.sqm_override = Some("cake diffserv3".to_string());
        let inputs = TopologyShapingInputsFile {
            circuits: vec![c],
            ..Default::default()
        };
        let tx = compile_dataplane_transaction_with_ranges_and_sqm(
            &network,
            &inputs,
            &[],
            "cake diffserv4",
        );
        assert_eq!(
            leaf_aqm(&tx, "008b8b44"),
            DataplaneLeafAqm::Cake {
                diffserv4: false,
                rtt_ms: None
            }
        );
    }

    /// A `none`/absent default stays FIFO.
    #[test]
    fn none_default_stays_fifo() {
        let network = effective_tree_with_parent_ceil(1_000, 1_000);
        let inputs = TopologyShapingInputsFile {
            circuits: vec![circuit("uisp:device:af8cae80", 200.0, 300.0)],
            ..Default::default()
        };
        let tx =
            compile_dataplane_transaction_with_ranges_and_sqm(&network, &inputs, &[], "none");
        assert_eq!(leaf_aqm(&tx, "008b8b44"), DataplaneLeafAqm::None);
    }

    /// `lqos-74t.150`: the v2 parser preserves `250.5` Mbps to the circuit row as
    /// `f32`; the dataplane then rounds to whole Mbps and encodes `u32` kbps.
    /// `250.5` -> `251` Mbps -> `251_000` kbps.
    #[test]
    fn rate_pair_encodes_v2_decimal_ceiling_as_kbps() {
        let pair = rate_pair(10.0, 250.5);
        assert_eq!(pair.commit_kbps, 10_000);
        assert_eq!(pair.ceil_kbps, 251_000);
    }
}


#[cfg(test)]
mod orphans_umbrella_tests {
    use super::*;

    /// `lqos-74t.15.2` (A1), SUPERSEDING `lqos-74t.3`: `circuit_bearing_nodes`
    /// KEEPS **EVERY** childless, circuit-less node (== the analysis's
    /// `walk_place:347` unit rule) — the former `orphans`-only exemption is
    /// SUBSUMED (the `ORPHANS_UMBRELLA_NODE_ID` const stays). A sibling
    /// circuit-less node (`empty-site`) is now ALSO kept.
    #[test]
    fn orphans_umbrella_and_childless_siblings_are_kept() {
        let effective = serde_json::json!({
            "Root Site": {
                "id": "root-site",
                "name": "Root Site",
                "children": {
                    "libreqos:generated:uisp:site:orphans": {
                        "id": lqos_dataplane_types::ORPHANS_UMBRELLA_NODE_ID,
                        "name": "Orphans",
                        "children": {}
                    },
                    "Empty Site": {
                        "id": "empty-site",
                        "name": "Empty Site",
                        "children": {}
                    }
                }
            }
        });
        let inputs = TopologyShapingInputsFile::default();
        let keep = circuit_bearing_nodes(&effective, &inputs, &[]);
        assert!(
            keep.contains(lqos_dataplane_types::ORPHANS_UMBRELLA_NODE_ID),
            "the orphans umbrella must always be kept"
        );
        assert!(
            keep.contains("root-site"),
            "the umbrella's ROOT ancestry rides keep_ancestry"
        );
        assert!(
            keep.contains("empty-site"),
            "(A1): a childless circuit-less sibling is KEPT too (the exemption is GENERAL now)"
        );
    }
}
