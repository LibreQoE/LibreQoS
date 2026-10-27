//! T1 — populate the dashboard tree store from the manager's E2
//! operator view (`state/topology/topology_operator_view.json`).
//!
//! Derivations: `docs/t1_tree_repoint_derivation.md` (fresh-#12 folded)
//! + T5 rung 1 (`docs/t5_topology_manager_consolidation_derivation.md`):
//!   the T1 builder repoints from the dataplane transaction to E2, which
//!   retires the T4 display-name side-channel in the same change (names
//!   ride E2 natively — never both transports live). E2 also keeps
//!   circuit-less sites the shaping projection (E1) elides, so the
//!   operator tree regains the full imported topology.
//!
//! Pinned contracts (all `[verdict]`-binding):
//! - HIGH-1 identity pin: store `name` := artifact `node_id`/`circuit_id`
//!   (the live join is NAME-keyed — `get_parents_for_circuit_id(&device.
//!   parent_node)`); device `parent_node` strings that match no store node
//!   are REPORTED (`divergent_parent_nodes`), never silent.
//! - MEDIUM-2 synthetic root: index 0, name exactly `"Root"`, `id: None`,
//!   `immediate_parent: None` (uniquely), `parents: []` (NOT
//!   self-inclusive at root), `max_throughput: (0,0)`, `node_type: None`;
//!   nodes BELOW root are self-inclusive (`parents.last() == own index`);
//!   emission is TOPOLOGICAL (parents before children — artifact op order
//!   is HashMap-derived and must not be trusted); artifact nodes with
//!   root-like names or dangling/unreachable parents are skipped (report
//!   + WARN at the caller).
//! - MEDIUM-3: circuit nodes are structure-only in store-joined surfaces;
//!   live circuit counters arrive only via the E2 `by_circuit_id` cache.
//! - LOW-6 None-policy: lat/long ARE carried from the E2 operator view (the
//!   site map depends on them); `active_attachment_name`
//!   is never populated from the artifact; runtime state (heatmaps,
//!   live counters) carries forward in `state::publish_network_json`.

use lqos_config::{NetworkJson, NetworkJsonNode, TopologyOperatorViewFile};
use lqos_dataplane_types::DATA_PLANE_ROOT_NODE_ID;
use lqos_utils::units::DownUpOrder;
use std::collections::{BTreeMap, BTreeSet};

/// The synthetic root's store name (MEDIUM-2: consumers string-match it).
pub(crate) const TREE_ROOT_NAME: &str = "Root";

/// The structure-less store published when no artifact has ever loaded
/// (T1b honest-empty posture): root only, zero topology. Root-only keeps
/// the index-0 invariants every consumer assumes.
pub(crate) fn structureless_tree() -> NetworkJson {
    let mut tree = NetworkJson::new();
    tree.nodes.push(zeroed_node(TREE_ROOT_NAME.to_string(), None));
    tree
}

/// Outcome of one artifact-tree build. Reports are returned, not just
/// logged, so the gates can assert them.
#[derive(Debug, Default)]
pub(crate) struct ArtifactTreeBuild {
    /// The populated store payload (root + topologically ordered nodes).
    pub tree: NetworkJson,
    /// Distinct device `parent_node` strings matching no store node
    /// (HIGH-1 divergence: those devices render zero live data).
    pub divergent_parent_nodes: Vec<String>,
    /// `node_id`/`circuit_id`s skipped for root-like names, dangling or
    /// unreachable parents.
    pub skipped_nodes: Vec<String>,
}

fn root_like(name: &str) -> bool {
    name.trim().eq_ignore_ascii_case(DATA_PLANE_ROOT_NODE_ID) || name.trim() == TREE_ROOT_NAME
}

fn zeroed_node(name: String, id: Option<String>) -> NetworkJsonNode {
    NetworkJsonNode {
        name,
        id,
        max_throughput: (0.0, 0.0),
        current_throughput: DownUpOrder::zeroed(),
        current_packets: DownUpOrder::zeroed(),
        current_tcp_packets: DownUpOrder::zeroed(),
        current_udp_packets: DownUpOrder::zeroed(),
        current_icmp_packets: DownUpOrder::zeroed(),
        current_tcp_retransmits: DownUpOrder::zeroed(),
        current_tcp_retransmit_packets: DownUpOrder::zeroed(),
        current_rtt_flows: DownUpOrder::zeroed(),
        current_marks: DownUpOrder::zeroed(),
        current_drops: DownUpOrder::zeroed(),
        rtt_buffer: Default::default(),
        parents: Vec::new(),
        immediate_parent: None,
        node_type: None,
        latitude: None,
        longitude: None,
        active_attachment_name: None,
        heatmap: None,
    }
}

/// Builds the store payload from the E2 operator view.
///
/// Pure: no IO, no global state, no logging (callers warn on the reports).
/// Deterministic: nodes are keyed by id and emitted in (depth, id) order,
/// so parents always precede children regardless of file order. T5 rung 1
/// repoint: names are native in E2 (the T4 side-channel retired) and
/// circuit-less sites ride along (the operator view is the FULL topology).
pub(crate) fn build_tree_from_operator_view(
    view: &TopologyOperatorViewFile,
    device_parent_nodes: &[String],
) -> ArtifactTreeBuild {
    let mut skipped: Vec<String> = Vec::new();

    // Node records keyed by id (file order must not matter).
    struct NodeRecord {
        parent_id: Option<String>,
        name: String,
        node_type: Option<String>,
        download_mbps: f64,
        upload_mbps: f64,
        latitude: Option<f32>,
        longitude: Option<f32>,
    }
    let mut records: BTreeMap<String, NodeRecord> = BTreeMap::new();
    for node in &view.nodes {
        if root_like(&node.id) || root_like(&node.name) {
            skipped.push(node.id.clone());
            continue;
        }
        if node.id.trim().is_empty() {
            skipped.push(node.id.clone());
            continue;
        }
        records.insert(
            node.id.clone(),
            NodeRecord {
                parent_id: node.parent_id.clone(),
                name: node.name.clone(),
                node_type: if node.node_type.trim().is_empty() {
                    None
                } else {
                    Some(node.node_type.to_ascii_lowercase())
                },
                download_mbps: node.download_mbps.unwrap_or(0.0),
                upload_mbps: node.upload_mbps.unwrap_or(0.0),
                // The operator view carries f64 (JSON-exact); this legacy
                // NodeRecord is f32 — a deliberate narrowing at the boundary.
                latitude: node.latitude.map(|value| value as f32),
                longitude: node.longitude.map(|value| value as f32),
            },
        );
    }

    // Depth per node via parent-walk with cycle rejection. A node is
    // emitted only if its ancestry terminates at the synthetic root
    // (parent_id None/empty = root-level).
    fn depth_of(id: &str, records: &BTreeMap<String, NodeRecord>) -> Option<usize> {
        let mut current = id.to_string();
        let mut depth = 0usize;
        let mut seen = BTreeSet::new();
        loop {
            let record = records.get(&current)?;
            depth += 1;
            if depth > records.len() {
                return None; // cycle
            }
            match record.parent_id.as_deref().map(str::trim) {
                None | Some("") => return Some(depth),
                Some(parent) => {
                    if !seen.insert(current.clone()) {
                        return None; // cycle
                    }
                    if records.contains_key(parent) {
                        current = parent.to_string();
                    } else {
                        return None; // dangling parent
                    }
                }
            }
        }
    }

    struct Emit {
        id: String,
        depth: usize,
    }
    let mut emittable: Vec<Emit> = Vec::new();
    for id in records.keys() {
        match depth_of(id, &records) {
            Some(depth) => emittable.push(Emit {
                id: id.clone(),
                depth,
            }),
            None => skipped.push(id.clone()),
        }
    }
    // (depth, id) order == topological + deterministic under shuffles.
    emittable.sort_by(|a, b| (a.depth, &a.id).cmp(&(b.depth, &b.id)));

    let mut tree = NetworkJson::new();
    // MEDIUM-2: the synthetic root, exactly as >=10 consumers hardcode it.
    tree.nodes.push(zeroed_node(TREE_ROOT_NAME.to_string(), None));

    let mut index_by_id: BTreeMap<String, usize> = BTreeMap::new();
    for emit in &emittable {
        let record = records
            .get(&emit.id)
            .expect("depth order guarantees the record exists");
        let (parent_idx, parent_is_root) = match record.parent_id.as_deref().map(str::trim) {
            None | Some("") => (0usize, true),
            Some(parent) => (
                *index_by_id
                    .get(parent)
                    .expect("depth order guarantees the parent is emitted"),
                false,
            ),
        };
        let idx = tree.nodes.len();
        let mut new_node = zeroed_node(record.name.clone(), Some(emit.id.clone()));
        // Legacy chain semantics (MEDIUM-2): every node below root is
        // self-inclusive — depth-1 carries [0, own]; deeper nodes carry
        // their parent's chain (already root-anchored) plus their own.
        new_node.parents = if parent_is_root {
            vec![0, idx]
        } else {
            let mut parents = tree.nodes[parent_idx].parents.clone();
            parents.push(idx);
            parents
        };
        new_node.immediate_parent = Some(parent_idx);
        new_node.max_throughput = (record.download_mbps, record.upload_mbps);
        new_node.node_type = record.node_type.clone();
 // Site-map coordinates: carried through the canonical
        // model → E2 operator view → here, so the site_map consumes them via
        // the existing tracker mapping. Producer-agnostic.
        new_node.latitude = record.latitude;
        new_node.longitude = record.longitude;
        tree.nodes.push(new_node);
        index_by_id.insert(emit.id.clone(), idx);
    }

    // HIGH-1 divergence report: device parent_node strings are NAME-keyed
    // into this store; a miss renders zero live data (asserted, not silent).
    let mut divergent: BTreeSet<String> = BTreeSet::new();
    for parent in device_parent_nodes {
        let trimmed = parent.trim();
        if trimmed.is_empty() {
            continue;
        }
        // T4 pin 2 (carried into E2): a raw key claimed by >1 node
        // (dedupe suffixed the siblings) is AMBIGUOUS — it must miss
        // the raw-name join LOUDLY (zero live data + the report),
        // never silently attach to whichever sibling kept the plain
        // name.
        if view.ambiguous_base_names.contains(trimmed) {
            divergent.insert(trimmed.to_string());
            continue;
        }
        if !tree.nodes.iter().any(|n| n.name == trimmed) {
            divergent.insert(trimmed.to_string());
        }
    }

    ArtifactTreeBuild {
        tree,
        divergent_parent_nodes: divergent.into_iter().collect(),
        skipped_nodes: skipped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lqos_config::{OperatorViewDevice, OperatorViewNode};

    const SITE_ID: &str = "libreqos:legacy-network-json:node:fc12ff8270a70366";
    const AP_ID: &str = "libreqos:legacy-network-json:node:c99483a36594a604";

    fn view_node(
        id: &str,
        name: &str,
        parent: Option<&str>,
        node_type: &str,
        download_mbps: f64,
        upload_mbps: f64,
    ) -> OperatorViewNode {
        OperatorViewNode {
            id: id.to_string(),
            name: name.to_string(),
            parent_id: parent.map(str::to_string),
            node_type: node_type.to_string(),
            download_mbps: Some(download_mbps),
            upload_mbps: Some(upload_mbps),
            commit_download_mbps: None,
            commit_upload_mbps: None,
            devices: Vec::new(),
            latitude: None,
            longitude: None,
            tunnel_identity: None,
            rate_source: lqos_config::OperatorRateSource::Base,
        }
    }

    #[test]
    fn build_tree_carries_site_coordinates_from_the_operator_view() {
 // the site map's coordinates must survive the
        // canonical -> E2 operator view -> T1 tree chain (the tracker then
        // maps them to the map). Producer-agnostic field carriage.
        let mut site = view_node(SITE_ID, "Tower A", None, "site", 100.0, 100.0);
        site.latitude = Some(45.5);
        site.longitude = Some(-111.25);
        let view = TopologyOperatorViewFile {
            schema_version: 1,
            source_generation: "test".to_string(),
            nodes: vec![site],
            ambiguous_base_names: Default::default(),
        };
        let build = build_tree_from_operator_view(&view, &[]);
        let node = build
            .tree
            .nodes
            .iter()
            .find(|n| n.id.as_deref() == Some(SITE_ID))
            .expect("site node should be emitted");
        assert_eq!(node.latitude, Some(45.5));
        assert_eq!(node.longitude, Some(-111.25));
    }

    /// `docs/topology_ptmp_ptp_attachment_card.md` §3.11: the store built from
    /// the operator view shows SITES UNDER APS and carries NO link/station radio
    /// node. The WS `NetworkJsonTransport` `type` is derived from this store, so
    /// a clean store is a clean payload — the link is an attachment edge only.
    #[test]
    fn tree_shows_sites_under_aps_and_no_link_radio_nodes() {
        const AP: &str = "uisp:device:jr-ap-tc-c";
        const CHILD: &str = "uisp:site:trailercity-tower-trailer";
        let view = TopologyOperatorViewFile {
            schema_version: 1,
            source_generation: "card".to_string(),
            nodes: vec![
                view_node("uisp:site:trailercity", "TrailerCity", None, "site", 1000.0, 1000.0),
                view_node(AP, "JR_AP_TC_C", Some("uisp:site:trailercity"), "ap", 1000.0, 1000.0),
                view_node(CHILD, "TrailerCity Tower Trailer", Some(AP), "site", 100.0, 100.0),
            ],
            ambiguous_base_names: Default::default(),
        };
        let build = build_tree_from_operator_view(&view, &[]);
        assert!(build.skipped_nodes.is_empty());
        assert!(
            !build
                .tree
                .nodes
                .iter()
                .any(|n| n.id.as_deref() == Some("uisp:device:wave-long-range")),
            "a link/station radio is absent from the store (the WS payload)"
        );
        let ap_idx = build
            .tree
            .nodes
            .iter()
            .position(|n| n.id.as_deref() == Some(AP))
            .expect("the serving AP is a node");
        assert_eq!(build.tree.nodes[ap_idx].node_type.as_deref(), Some("ap"));
        let child = build
            .tree
            .nodes
            .iter()
            .find(|n| n.id.as_deref() == Some(CHILD))
            .expect("the child site is a node");
        assert_eq!(
            child.immediate_parent,
            Some(ap_idx),
            "the site renders UNDER the AP (sites under APs)"
        );
        assert_eq!(child.node_type.as_deref(), Some("site"));

 // the implementer `#12` R7: assert on the EXACT served payload the WS `NetworkMap`
        // ships — `NetworkJsonTransport`, whose kind field is `node_type`
        // (serde `"type"`). The link must be ABSENT (no link renders ANY type);
        // the SITE still serializes `type=site` and the AP `type=ap`.
        let transports: Vec<serde_json::Value> = build
            .tree
            .nodes
            .iter()
            .map(|node| serde_json::to_value(node.clone_to_transit()).expect("transport serializes"))
            .collect();
        assert!(
            transports.iter().all(|t| {
                t.get("id").and_then(|v| v.as_str()) != Some("uisp:device:wave-long-range")
            }),
            "the link is absent from the served payload"
        );
        let kind = |id: &str| -> Option<String> {
            transports
                .iter()
                .find(|t| t.get("id").and_then(|v| v.as_str()) == Some(id))
                .and_then(|t| t.get("type").and_then(|v| v.as_str()).map(str::to_string))
        };
        assert_eq!(kind(AP).as_deref(), Some("ap"), "the AP serializes `type=ap`");
        assert_eq!(
            kind(CHILD).as_deref(),
            Some("site"),
            "the site serializes `type=site`"
        );
    }

    /// The rig world: Root → s10-a → s10-a-ap0 → c1, plus the
    /// circuit-less s10-b site the shaping projection (E1) elides —
    /// the operator view (E2) KEEPS it (F-LM §8-4 resolved).
    fn rig_view() -> TopologyOperatorViewFile {
        let mut view = TopologyOperatorViewFile {
            schema_version: 1,
            source_generation: "t5-gen".to_string(),
            nodes: vec![
                view_node(SITE_ID, "s10-a", None, "site", 10_000.0, 10_000.0),
                view_node(AP_ID, "s10-a-ap0", Some(SITE_ID), "ap", 10_000.0, 10_000.0),
                view_node("c1", "Bench Client 1", Some(AP_ID), "circuit", 200.0, 100.0),
                // Deliberately NOT id-sorted in this fixture list — the
                // builder must not trust file order.
                view_node(
                    "libreqos:legacy-network-json:node:3a05f3f9c95a92be",
                    "s10-b",
                    None,
                    "site",
                    0.0,
                    0.0,
                ),
            ],
            ambiguous_base_names: Default::default(),
        };
        view.nodes[2].devices = vec![OperatorViewDevice {
            device_id: "dev1".to_string(),
            device_name: "Bench Client 1".to_string(),
            ipv4: vec!["100.97.0.1".to_string()],
            ..Default::default()
        }];
        view
    }

    /// The store contract over a populated world: topological
    /// emission, human display names NATIVE (T5 rung 1 — the T4
    /// side-channel retired), artifact ids in the id slot (T1c/pin 4),
    /// rates and types carried, circuit-less sites present.
    #[test]
    fn builds_tree_from_operator_view_with_human_names() {
        let build = build_tree_from_operator_view(&rig_view(), &[]);
        let nodes = &build.tree.nodes;
        assert_eq!(nodes.len(), 5, "root + site + ap + circuit + empty site");
        assert!(build.skipped_nodes.is_empty());

        let root = &nodes[0];
        assert_eq!(root.name, "Root");
        assert_eq!(root.id, None);
        // (depth, id) order: s10-b's id sorts before s10-a's at depth 1.
        let empty_site = &nodes[1];
        assert_eq!(
            empty_site.name, "s10-b",
            "the elided-in-E1 empty site rides in E2"
        );
        assert_eq!(empty_site.node_type.as_deref(), Some("site"));
        assert_eq!(empty_site.immediate_parent, Some(0));
        let site = &nodes[2];
        assert_eq!(site.name, "s10-a");
        assert_eq!(site.id.as_deref(), Some(SITE_ID));
        assert_eq!(site.node_type.as_deref(), Some("site"));
        assert_eq!(site.max_throughput, (10_000.0, 10_000.0));
        let ap = &nodes[3];
        assert_eq!(ap.name, "s10-a-ap0");
        assert_eq!(ap.id.as_deref(), Some(AP_ID));
        assert_eq!(ap.node_type.as_deref(), Some("ap"));
        assert_eq!(ap.immediate_parent, Some(2));
        let circuit = &nodes[4];
        assert_eq!(circuit.name, "Bench Client 1");
        assert_eq!(circuit.id.as_deref(), Some("c1"));
        assert_eq!(circuit.node_type.as_deref(), Some("circuit"));
        assert_eq!(circuit.immediate_parent, Some(3));
        assert_eq!(circuit.max_throughput, (200.0, 100.0));
    }

    /// Shuffled FILE order must not matter: the builder emits in
    /// (depth, id) order from keyed records.
    #[test]
    fn file_order_is_deterministic() {
        let ordered = build_tree_from_operator_view(&rig_view(), &[]);
        let mut shuffled = rig_view();
        shuffled.nodes.reverse();
        let reordered = build_tree_from_operator_view(&shuffled, &[]);

        assert_eq!(ordered.tree.nodes.len(), reordered.tree.nodes.len());
        for (o, r) in ordered.tree.nodes.iter().zip(reordered.tree.nodes.iter()) {
            assert_eq!(o.name, r.name);
            assert_eq!(o.id, r.id);
            assert_eq!(o.immediate_parent, r.immediate_parent);
            assert_eq!(o.parents, r.parents);
        }
    }

    /// Dangling parents, cycles, and root-like names are SKIPPED with
    /// a report — never a silent drop of the whole build.
    #[test]
    fn dangling_root_like_and_cyclic_nodes_are_skipped() {
        let mut view = rig_view();
        view.nodes.push(view_node(
            "c-dangling",
            "Dangling Circuit",
            Some("libreqos:missing-node"),
            "circuit",
            10.0,
            10.0,
        ));
        view.nodes.push(view_node("real", "Real Site", None, "site", 5.0, 5.0));
        view.nodes.push(view_node("Root", "Root Imposter", None, "site", 1.0, 1.0));
        // A cycle: two sites claiming each other as parents.
        view.nodes.push(view_node("cyc-a", "Cycle A", Some("cyc-b"), "site", 1.0, 1.0));
        view.nodes.push(view_node("cyc-b", "Cycle B", Some("cyc-a"), "site", 1.0, 1.0));

        let build = build_tree_from_operator_view(&view, &[]);
        let names: Vec<&str> = build
            .tree
            .nodes
            .iter()
            .map(|n| n.name.as_str())
            .collect();
        assert!(names.contains(&"Root"));
        assert!(names.contains(&"s10-a"));
        assert!(names.contains(&"Real Site"));
        assert_eq!(build.skipped_nodes.len(), 4, "{:?}", build.skipped_nodes);
        assert!(build.skipped_nodes.contains(&"Root".to_string()));
        assert!(build.skipped_nodes.contains(&"c-dangling".to_string()));
        assert!(build.skipped_nodes.contains(&"cyc-a".to_string()));
        assert!(build.skipped_nodes.contains(&"cyc-b".to_string()));
    }

    /// HIGH-1 carried into E2: device parent strings that match no
    /// store node name are REPORTED (loud zero live data), blanks are
    /// ignored.
    #[test]
    fn divergent_device_parents_stay_loud() {
        let device_parents = vec![
            "ghost-site".to_string(),
            "  ".to_string(),
            "s10-a-ap0".to_string(),
        ];
        let build = build_tree_from_operator_view(&rig_view(), &device_parents);
        assert_eq!(build.divergent_parent_nodes, vec!["ghost-site".to_string()]);
    }

    /// T4 pin 2 carried into E2: a raw base claimed by two nodes
    /// misses the raw-name join LOUDLY while the deduped suffix
    /// joins.
    #[test]
    fn ambiguous_base_names_miss_loudly_suffix_hits() {
        const SITE_B_ID: &str = "libreqos:legacy-network-json:node:2222222222222222";
        let mut view = rig_view();
        view.nodes.push(view_node(
            SITE_B_ID,
            "tower-1 [22222222]",
            None,
            "site",
            10.0,
            10.0,
        ));
        // Both claimants carry SUFFIXED store names (the dedupe
        // assigned no plain owner) and the base is ambiguous.
        view.nodes[0].name = "tower-1 [fc12ff82]".to_string();
        view.ambiguous_base_names.insert("tower-1".to_string());

        let build =
            build_tree_from_operator_view(&view, &["tower-1".to_string()]);
        assert!(
            build.divergent_parent_nodes.contains(&"tower-1".to_string()),
            "the ambiguous RAW base misses LOUDLY"
        );

        let deduped_names = vec![
            "tower-1 [fc12ff82]".to_string(),
            "tower-1 [22222222]".to_string(),
        ];
        let build = build_tree_from_operator_view(&view, &deduped_names);
        assert!(
            build.divergent_parent_nodes.is_empty(),
            "deduped store names join natively: {:?}",
            build.divergent_parent_nodes
        );
    }

    /// F10 (zero-CSV UISP world): circuit identity rides INLINE on the
    /// E2 circuit node — the tree build itself stays structure-only
    /// (MEDIUM-3) while the catalog arm consumes the same records.
    #[test]
    fn circuit_nodes_carry_inline_devices_in_the_view() {
        let view = rig_view();
        let circuit = view.nodes.iter().find(|n| n.id == "c1").unwrap();
        assert_eq!(circuit.devices.len(), 1);
        assert_eq!(circuit.devices[0].device_name, "Bench Client 1");
        assert_eq!(circuit.devices[0].ipv4, vec!["100.97.0.1".to_string()]);
        // The store row stays structure-only (live data via E2 cache).
        let build = build_tree_from_operator_view(&view, &[]);
        let row = build
            .tree
            .nodes
            .iter()
            .find(|n| n.id.as_deref() == Some("c1"))
            .expect("circuit row present");
        assert_eq!(row.current_throughput.down, 0);
    }
}
