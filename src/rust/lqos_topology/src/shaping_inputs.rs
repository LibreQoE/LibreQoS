fn collect_exported_effective_nodes(
    value: &Value,
    by_id: &mut HashMap<String, String>,
    unique_names: &mut HashMap<String, Option<String>>,
    ambiguous_names: &mut HashSet<String>,
) {
    let Some(nodes) = value.as_object() else {
        return;
    };
    for (key, node) in nodes {
        let Some(map) = node.as_object() else {
            continue;
        };
        let node_id = map
            .get("id")
            .and_then(Value::as_str)
            .and_then(optional_non_empty);
        let node_name = map
            .get("name")
            .and_then(Value::as_str)
            .and_then(optional_non_empty)
            .or_else(|| optional_non_empty(key));
        if let Some(node_name) = node_name {
            if let Some(node_id) = node_id.clone() {
                by_id.insert(node_id, node_name.clone());
            }
            if !ambiguous_names.contains(&node_name) {
                match unique_names.entry(node_name) {
                    Entry::Vacant(entry) => {
                        entry.insert(node_id);
                    }
                    Entry::Occupied(entry) => {
                        let (duplicate_name, _) = entry.remove_entry();
                        ambiguous_names.insert(duplicate_name);
                    }
                }
            }
        }
        if let Some(children) = map.get("children") {
            collect_exported_effective_nodes(children, by_id, unique_names, ambiguous_names);
        }
    }
}

fn collect_exported_effective_aliases(
    value: &Value,
    aliases: &mut HashMap<String, (String, String)>,
) {
    let Some(nodes) = value.as_object() else {
        return;
    };
    for (key, node) in nodes {
        let Some(map) = node.as_object() else {
            continue;
        };
        let node_id = map
            .get("id")
            .and_then(Value::as_str)
            .and_then(optional_non_empty);
        let node_name = map
            .get("name")
            .and_then(Value::as_str)
            .and_then(optional_non_empty)
            .or_else(|| optional_non_empty(key));
        let active_attachment_name = map
            .get("active_attachment_name")
            .and_then(Value::as_str)
            .and_then(optional_non_empty);
        if let (Some(alias), Some(node_id), Some(node_name)) =
                (active_attachment_name, node_id, node_name)
        {
            aliases.entry(alias).or_insert((node_id, node_name));
        }
        if let Some(children) = map.get("children") {
            collect_exported_effective_aliases(children, aliases);
        }
    }
}

fn build_effective_queue_aliases(
    ui_state: &TopologyEditorStateFile,
    effective: &TopologyEffectiveStateFile,
    exported_effective_nodes: &HashMap<String, String>,
) -> (EffectiveQueueAliasMap, EffectiveQueueAliasMap) {
    let ui_by_node = ui_state
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let logical_parent_by_node = effective
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node.logical_parent_node_id.as_str()))
        .collect::<HashMap<_, _>>();
    let mut aliases_by_id = HashMap::new();
    let mut aliases_by_name = HashMap::new();

    for effective_node in &effective.nodes {
        if exported_effective_nodes.contains_key(&effective_node.node_id) {
            continue;
        }
        let Some(resolved) = nearest_exported_effective_parent(
            &effective_node.logical_parent_node_id,
            &logical_parent_by_node,
            exported_effective_nodes,
        ) else {
            continue;
        };
        aliases_by_id
            .entry(effective_node.node_id.clone())
            .or_insert_with(|| resolved.clone());
        if let Some(ui_node) = ui_by_node.get(effective_node.node_id.as_str()).copied() {
            aliases_by_name
                .entry(ui_node.node_name.clone())
                .or_insert_with(|| resolved.clone());
        }
    }

    (aliases_by_id, aliases_by_name)
}

fn nearest_exported_effective_parent(
    parent_id: &str,
    logical_parent_by_node: &HashMap<&str, &str>,
    exported_effective_nodes: &HashMap<String, String>,
) -> Option<(String, String)> {
    let mut cursor = parent_id.trim();
    let mut seen = HashSet::new();
    while !cursor.is_empty() && seen.insert(cursor.to_string()) {
        if let Some(parent_name) = exported_effective_nodes.get(cursor) {
            return Some((cursor.to_string(), parent_name.clone()));
        }
        cursor = logical_parent_by_node
            .get(cursor)
            .copied()
            .unwrap_or_default();
    }
    None
}

fn effective_parent_is_exported(
    parent_id: &str,
    parent_name: &str,
    exported_effective_nodes: &HashMap<String, String>,
    exported_effective_names: &HashMap<String, Option<String>>,
) -> bool {
    optional_non_empty(parent_id)
        .is_some_and(|node_id| exported_effective_nodes.contains_key(&node_id))
        || optional_non_empty(parent_name)
            .is_some_and(|node_name| exported_effective_names.contains_key(&node_name))
        // The dataplane root is IMPLICIT: no effective network can export a node
        // object for it, yet the data plane always carries it (tree index 0). A
        // row attached to it is exported by definition — its empty parent id is
        // the wire's root sentinel.
        || optional_non_empty(parent_name)
            .as_deref()
            .is_some_and(is_dataplane_root_name)
}

/// Whether a resolved parent NAME is the dataplane root.
fn is_dataplane_root_name(name: &str) -> bool {
    name.eq_ignore_ascii_case(DATA_PLANE_ROOT_NAME)
}

/// Whether one store row carries the fold's OWN root convention: no parent id AND
/// a blank or `Root` parent name (`u15` A2).
///
/// Blank and `Root` are ONE predicate by spec (driver ruling 2026-11-17): "if no
/// `attach_to` is specified, you attach to the top of the tree", and the actor
/// writes [`DATA_PLANE_ROOT_NAME`] for exactly that case, so a row with no parent
/// information at all and a row naming the sentinel are the same instruction.
///
/// The id half is what keeps A2 from widening: a row that DOES carry an id keeps
/// the existing pre-check and id-shaped resolution, so a stale id still defers
/// instead of being read as the root. An id that is present but blank counts as
/// absent, the same way the rest of the fold reads `parent_node_id`.
fn is_root_sentinel_row(device: &lqos_config::ShapedDevice) -> bool {
    let carries_id = device
        .parent_node_id
        .as_deref()
        .map(str::trim)
        .is_some_and(|id| !id.is_empty());
    let name = device.parent_node.trim();
    !carries_id && (name.is_empty() || is_dataplane_root_name(name))
}

fn resolve_legacy_parent_from_effective_tree(
    parent_node: &str,
    parent_node_id: Option<&str>,
    exported_effective_nodes: &HashMap<String, String>,
    exported_effective_names: &HashMap<String, Option<String>>,
    exported_effective_aliases: &HashMap<String, (String, String)>,
    queue_aliases_by_id: &HashMap<String, (String, String)>,
    queue_aliases_by_name: &HashMap<String, (String, String)>,
) -> Option<(String, String)> {
    let trimmed_id = parent_node_id.and_then(optional_non_empty);
    let trimmed_name = optional_non_empty(parent_node);

    if let Some(parent_id) = trimmed_id.as_deref()
        && let Some(parent_name) = exported_effective_nodes.get(parent_id).cloned()
    {
        return Some((parent_id.to_string(), parent_name));
    }
    if let Some(parent_id) = trimmed_id.as_deref()
        && let Some(resolved) = queue_aliases_by_id.get(parent_id).cloned()
    {
        return Some(resolved);
    }
    if let Some(parent_name) = trimmed_name.as_deref()
        && let Some(parent_id) = exported_effective_names.get(parent_name)
    {
        return Some((
            parent_id.clone().unwrap_or_default(),
            parent_name.to_string(),
        ));
    }
    if let Some(parent_name) = trimmed_name.as_deref()
        && let Some(resolved) = queue_aliases_by_name.get(parent_name).cloned()
    {
        return Some(resolved);
    }
    if let Some(resolved) = trimmed_name
        .as_deref()
        .and_then(|alias| exported_effective_aliases.get(alias).cloned())
    {
        return Some(resolved);
    }
    // LAST, so a REAL node named `Root` still wins: the implicit root is the
    // fallback every export-based lookup above has already declined.
    trimmed_name
        .filter(|name| is_dataplane_root_name(name))
        .map(|name| (String::new(), name))
}

fn selected_attachment_name_for_node(
    ui_node: &TopologyEditorNode,
    effective_node: &TopologyEffectiveNodeState,
) -> Option<String> {
    effective_node
        .effective_attachment_id
        .as_deref()
        .and_then(|attachment_id| {
            ui_node
                .allowed_parents
                .iter()
                .find_map(|parent| option_name(parent, attachment_id))
        })
        .or_else(|| optional_non_empty_owned(ui_node.effective_attachment_name.clone()))
        .or_else(|| optional_non_empty_owned(ui_node.current_attachment_name.clone()))
}

fn build_attachment_owner_map(
    ui_state: &TopologyEditorStateFile,
) -> HashMap<String, (String, String)> {
    let mut owners = HashMap::new();
    for node in &ui_state.nodes {
        for parent in &node.allowed_parents {
            for option in &parent.attachment_options {
                let Some(attachment_id) = optional_non_empty(&option.attachment_id) else {
                    continue;
                };
                if attachment_id == TOPOLOGY_ATTACHMENT_AUTO_ID {
                    continue;
                }
                owners
                    .entry(attachment_id)
                    .or_insert_with(|| (node.node_id.clone(), node.node_name.clone()));
            }
        }
    }
    owners
}

fn resolve_effective_parent_from_anchor(
    anchor_id: &str,
    ui_by_node: &HashMap<&str, &TopologyEditorNode>,
    effective_by_node: &HashMap<&str, &TopologyEffectiveNodeState>,
    exported_effective_nodes: &HashMap<String, String>,
    attachment_owner_by_attachment_id: &HashMap<String, (String, String)>,
    queue_aliases_by_id: &HashMap<String, (String, String)>,
) -> Option<(String, String, Option<String>, Option<String>)> {
    let anchor_id = anchor_id.trim();
    if anchor_id.is_empty() {
        return None;
    }

    if let Some(parent_name) = exported_effective_nodes.get(anchor_id).cloned() {
        let attachment_id = effective_by_node
            .get(anchor_id)
            .and_then(|node| optional_non_empty_owned(node.effective_attachment_id.clone()));
        let attachment_name = ui_by_node.get(anchor_id).and_then(|ui_node| {
            effective_by_node.get(anchor_id).and_then(|effective_node| {
                selected_attachment_name_for_node(ui_node, effective_node)
            })
        });
        return Some((
            anchor_id.to_string(),
            parent_name,
            attachment_id,
            attachment_name,
        ));
    }
    let Some((owner_node_id, _owner_node_name)) =
        attachment_owner_by_attachment_id.get(anchor_id).cloned()
    else {
        if let Some((parent_id, parent_name)) = queue_aliases_by_id.get(anchor_id).cloned() {
            return Some((parent_id, parent_name, None, None));
        }
        return None;
    };
    let owner_ui = ui_by_node.get(owner_node_id.as_str()).copied()?;
    let owner_effective = effective_by_node.get(owner_node_id.as_str()).copied()?;

    if let Some(selected_attachment_id) = owner_effective.effective_attachment_id.as_deref()
        && let Some(parent_name) = exported_effective_nodes
            .get(selected_attachment_id)
            .cloned()
    {
        return Some((
            selected_attachment_id.to_string(),
            parent_name,
            optional_non_empty(selected_attachment_id),
            selected_attachment_name_for_node(owner_ui, owner_effective),
        ));
    }

    if let Some(parent_name) = exported_effective_nodes
        .get(owner_node_id.as_str())
        .cloned()
    {
        return Some((
            owner_node_id,
            parent_name,
            optional_non_empty_owned(owner_effective.effective_attachment_id.clone()),
            selected_attachment_name_for_node(owner_ui, owner_effective),
        ));
    }
    if let Some((parent_id, parent_name)) = queue_aliases_by_id.get(owner_node_id.as_str()).cloned()
    {
        return Some((
            parent_id,
            parent_name,
            optional_non_empty_owned(owner_effective.effective_attachment_id.clone()),
            selected_attachment_name_for_node(owner_ui, owner_effective),
        ));
    }

    None
}

fn duplicate_circuit_shape_conflicts(
    circuit: &TopologyShapingCircuitInput,
    device: &lqos_config::ShapedDevice,
) -> Vec<&'static str> {
    let mut conflicts = Vec::new();
    if circuit.download_min_mbps != device.download_min_mbps {
        conflicts.push("Download Min Mbps");
    }
    if circuit.upload_min_mbps != device.upload_min_mbps {
        conflicts.push("Upload Min Mbps");
    }
    if circuit.download_max_mbps != device.download_max_mbps {
        conflicts.push("Download Max Mbps");
    }
    if circuit.upload_max_mbps != device.upload_max_mbps {
        conflicts.push("Upload Max Mbps");
    }
    if circuit.comment != device.comment {
        conflicts.push("Comment");
    }
    if circuit.sqm_override != device.sqm_override {
        conflicts.push("sqm");
    }
    conflicts
}

/// One mapping as the artifact spells it: a bare address for a host prefix, CIDR
/// otherwise (a v4 `/32` and a v6 `/128` both print bare).
fn prefix_to_string(entry: &Prefix) -> String {
    let width = if entry.0.is_ipv4() { 32 } else { 128 };
    if entry.1 >= width {
        entry.0.to_string()
    } else {
        format!("{}/{}", entry.0, entry.1)
    }
}

fn load_integration_shaping_artifacts(
    config: &Config,
) -> Result<
    Option<(
        ConfigShapedDevices,
        Vec<CircuitAnchor>,
        lqos_config::TopologyCircuitMatchIndex,
    )>,
> {
    let topology_import = TopologyImportFile::load(config)
        .with_context(|| "Unable to load topology_import.json while validating shaping ingress")?;
    let Some(compiled_shaping) = TopologyCompiledShapingFile::load(config).with_context(
        || "Unable to load topology_compiled_shaping.json while building shaping_inputs.json",
    )?
    else {
        return Ok(None);
    };
    let import_identity = topology_import
        .as_ref()
        .and_then(|file| file.ingress_identity.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let compiled_identity = compiled_shaping
        .ingress_identity
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let (Some(import_identity), Some(compiled_identity)) = (import_identity, compiled_identity)
        && import_identity != compiled_identity
    {
        anyhow::bail!(
            "Integration compiled shaping ingress identity '{}' did not match topology import identity '{}'",
            compiled_identity,
            import_identity
        );
    }
    Ok(Some({
        let artifacts = compiled_shaping.shaping_artifacts();
        (
            artifacts.shaped_devices,
            artifacts.circuit_anchors,
            artifacts.circuit_matches,
        )
    }))
}

/// What one dynamic-store fold contributed to the shaping rows.
struct DynamicFoldOutcome {
    /// LOUD warnings for rejected/deferred circuits.
    warnings: Vec<String>,
    /// The device ROWS the fold actually synthesized, keyed
    /// `(normalized circuit id, device id)`. These are the row origins the
    /// `dynamic` marker reports, and the rows the supersede rule must never
    /// treat as provisioned devices.
    origin_rows: lqos_utils::DynamicOriginRows,
}

/// K1 §1.1/§1.2: fold the RADIUS dynamic-circuit store into the base rows at
/// the DEVICE-ROW level (pre-circuit-loop), so the compiler emits per-IP
/// `Map` ops. Returns LOUD warnings for rejected/deferred rows, plus the device
/// ROWS it synthesized (the `dynamic` marker's source of truth):
/// - a `circuit_id` colliding with a provisioned circuit is REJECTED (a RADIUS
///   circuit never shadows a provisioned one);
/// - an unresolvable parent is `shaping_deferred` (kept in the store for the
///   audit surface, never silently root-placed or dropped).
///
/// K1 §7 amendment (the coordinator rev 594): the deferral generalizes to the
/// fallback-supplied parent/anchor ids. A RADIUS fallback that carries a
/// literal node NAME (or any id that does not land an EXISTING effective node,
/// attachment owner, or queue alias) is `shaping_deferred` — an empty-parent
/// emission is never legal (the bridge rejects it as "parent does not exist in
/// the applied tree").
///
/// `u15` A2 (driver ruling 2026-11-17): a row with NO parent id whose parent
/// name is blank or the fold's [`DATA_PLANE_ROOT_NAME`] sentinel is the root
/// convention — "if no `attach_to` is specified, you attach to the top of the
/// tree". It emits the WIRE ROOT exactly as a blank-`attach_to` range fallback
/// does (name `Root`, no id, normalized to `root` at the boundary), so the
/// promoted `/32` compiles a leaf in every topology — single-root, forest and
/// flat-bucket alike — instead of deferring on every publish. A row that carries
/// an id is not the sentinel and still defers loudly when that id cannot be
/// proven.
#[allow(clippy::too_many_arguments)]
fn fold_dynamic_circuits_into_devices(
    config: &Config,
    shaped_devices: &mut ConfigShapedDevices,
    exported_effective_nodes: &HashMap<String, String>,
    exported_effective_names: &HashMap<String, Option<String>>,
    exported_effective_aliases: &HashMap<String, (String, String)>,
    attachment_owner_by_attachment_id: &HashMap<String, (String, String)>,
    queue_aliases_by_id: &HashMap<String, (String, String)>,
    queue_aliases_by_name: &HashMap<String, (String, String)>,
) -> DynamicFoldOutcome {
    let mut warnings = Vec::new();
    let mut origin_rows = lqos_utils::DynamicOriginRows::new();
    let store = lqos_config::DynamicCircuitsFile::load(config);
    if store.circuits.is_empty() {
        return DynamicFoldOutcome {
            warnings,
            origin_rows,
        };
    }
    let provisioned: HashSet<String> = shaped_devices
        .devices
        .iter()
        .map(|device| lqos_utils::normalize_circuit_id_key(&device.circuit_id))
        .collect();
    let mut rows: Vec<lqos_config::ShapedDevice> = Vec::new();
    for circuit in store.circuits {
        let mut device = circuit.shaped;
        let circuit_key = lqos_utils::normalize_circuit_id_key(&device.circuit_id);
        if circuit_key.is_empty() {
            warnings.push(
                "a dynamic circuit with an empty circuit_id was ignored".to_string(),
            );
            continue;
        }
        if provisioned.contains(&circuit_key) {
            warnings.push(format!(
                "dynamic circuit '{}' collides with a provisioned circuit; rejected \
                 (a RADIUS circuit never shadows a provisioned one)",
                device.circuit_id
            ));
            continue;
        }
        // K1 §7: the fallback-supplied parent/anchor ids must land an existing
        // effective node, attachment owner, or queue alias. A name/typo/stale
        // id would otherwise emit an empty parent (the bridge's typed reject).
        if !fallback_identity_resolves(
            device.parent_node_id.as_deref(),
            exported_effective_nodes,
            attachment_owner_by_attachment_id,
            queue_aliases_by_id,
        ) || !fallback_identity_resolves(
            device.anchor_node_id.as_deref(),
            exported_effective_nodes,
            attachment_owner_by_attachment_id,
            queue_aliases_by_id,
        ) {
            warnings.push(format!(
                "dynamic circuit '{}' has an unresolvable fallback parent/anchor id \
                 (shaping_deferred; the store keeps it for the audit surface)",
                device.circuit_id
            ));
            continue;
        }
        // u15 A2 + driver ruling (2026-11-17): the fold's root sentinel IS the
        // WIRE ROOT. The dataplane root is IMPLICIT — it never appears in the
        // exported effective network, so there is no id to prove and no node to
        // resolve — and the root attachment is therefore the SAME empty-parent
        // emission a blank-`attach_to` range fallback already makes: name
        // `Root`, no id, normalized to `root` at the wire boundary. That is
        // topology-independent by construction (single-root, forest and
        // flat-bucket worlds emit it identically) and makes the promoted `/32`
        // the fallback prefix's SIBLING.
        //
        // A row that carries an id is NOT the sentinel: it keeps the id-shaped
        // resolution below, so a stale id still defers loudly instead of being
        // read as the root.
        if is_root_sentinel_row(&device) {
            device.parent_node = DATA_PLANE_ROOT_NAME.to_string();
            device.parent_node_id = None;
        } else {
            match resolve_legacy_parent_from_effective_tree(
                device.parent_node.trim(),
                device.parent_node_id.as_deref(),
                exported_effective_nodes,
                exported_effective_names,
                exported_effective_aliases,
                queue_aliases_by_id,
                queue_aliases_by_name,
            ) {
                Some((parent_id, parent_name)) if !parent_id.trim().is_empty() => {
                    device.parent_node = parent_name;
                    device.parent_node_id = Some(parent_id);
                }
                _ => {
                    warnings.push(format!(
                        "dynamic circuit '{}' has an unresolvable parent '{}' \
                         (shaping_deferred; the store keeps it for the audit surface)",
                        device.circuit_id, device.parent_node
                    ));
                    continue;
                }
            }
        }
        device.circuit_hash = lqos_utils::hash_to_i64(&device.circuit_id);
        device.device_hash = lqos_utils::hash_to_i64(&device.device_id);
        device.parent_hash = lqos_utils::hash_to_i64(&device.parent_node);
        origin_rows.insert_row(&device.circuit_id, &device.device_id);
        rows.push(device);
    }
    if !rows.is_empty() {
        let mut all = shaped_devices.devices.clone();
        all.extend(rows);
        shaped_devices.replace_with_new_data(all);
    }
    DynamicFoldOutcome {
        warnings,
        origin_rows,
    }
}

/// The range rules that are LIVE: the dynamic feature AND unknown-IP promotion
/// must both be on — the gate the retired compiler-only branch applied.
fn live_range_rules(config: &Config) -> &[lqos_config::DynamicCircuitRangeRule] {
    config
        .dynamic_circuits
        .as_ref()
        .filter(|cfg| cfg.enabled && cfg.enable_unknown_ip_promotion)
        .map(|cfg| cfg.ranges.as_slice())
        .unwrap_or_default()
}

/// One CIDR block: a network address and its prefix length, the currency the
/// ignore-subnet arithmetic works in.
type Prefix = (IpAddr, u32);

/// The address bits of `address` and the width of its family (32 or 128).
fn address_space(address: IpAddr) -> (u128, u32) {
    match address {
        IpAddr::V4(address) => (u128::from(u32::from_be_bytes(address.octets())), 32),
        IpAddr::V6(address) => (u128::from_be_bytes(address.octets()), 128),
    }
}

/// Rebuilds an address of `width` bits from its numeric value.
fn address_from_space(bits: u128, width: u32) -> IpAddr {
    if width == 32 {
        IpAddr::from((bits as u32).to_be_bytes())
    } else {
        IpAddr::from(bits.to_be_bytes())
    }
}

/// `prefix` with its host bits cleared. `None` when its length exceeds its
/// family width: that is not a prefix, so it never becomes coverage.
fn normalize_prefix((address, prefix_len): Prefix) -> Option<Prefix> {
    let (bits, width) = address_space(address);
    let host_bits = width.checked_sub(prefix_len)?;
    let network_bits = if host_bits == width {
        0
    } else {
        bits >> host_bits << host_bits
    };
    Some((address_from_space(network_bits, width), prefix_len))
}

/// Whether `outer` covers every address of `inner`. Both are normalized, and a
/// cross-family prefix never contains another (`0.0.0.0/0` is not all of IPv6).
fn prefix_contains(outer: Prefix, inner: Prefix) -> bool {
    let (outer_bits, width) = address_space(outer.0);
    let (inner_bits, inner_width) = address_space(inner.0);
    if width != inner_width || inner.1 > inner_width || inner.1 < outer.1 {
        return false;
    }
    let Some(host_bits) = width.checked_sub(outer.1) else {
        return false;
    };
    host_bits == width || outer_bits >> host_bits == inner_bits >> host_bits
}

/// The two halves of `prefix`, one bit longer. `None` for a host prefix (a /32
/// or /128 has no halves) or a length its family cannot hold.
fn split_prefix((address, prefix_len): Prefix) -> Option<[Prefix; 2]> {
    let (bits, width) = address_space(address);
    let host_bits = width.checked_sub(prefix_len)?;
    let step = 1u128 << host_bits.checked_sub(1)?;
    Some([
        (address_from_space(bits, width), prefix_len + 1),
        (address_from_space(bits + step, width), prefix_len + 1),
    ])
}

/// `rule` minus `ignored`: the disjoint fragments covering exactly the rule's
/// addresses that no same-family ignore covers, sorted and deduplicated.
///
/// Prefix arithmetic, never per-address enumeration. Two prefixes of one family
/// are either disjoint or nested, so there are only three cases per fragment: it
/// avoids every ignore and survives whole, it is contained by one and is
/// dropped, or it contains one and is bisected until each half avoids or is
/// contained. Inputs are normalized first, so `10.1.2.3/16` means its network.
///
/// `None` when an input length exceeds its family width: the caller must refuse
/// the fallback rather than publish unfiltered or partial coverage. An EMPTY
/// vector is a real answer — the rule is entirely ignored.
fn subtract_ignored_prefixes(rule: Prefix, ignored: &[Prefix]) -> Option<Vec<Prefix>> {
    let rule = normalize_prefix(rule)?;
    let ignored = ignored
        .iter()
        .copied()
        .map(normalize_prefix)
        .collect::<Option<Vec<_>>>()?;
    let mut fragments = Vec::new();
    let mut pending = vec![rule];
    while let Some(fragment) = pending.pop() {
        let intersecting = ignored
            .iter()
            .copied()
            .find(|ignore| prefix_contains(*ignore, fragment) || prefix_contains(fragment, *ignore));
        match intersecting {
            None => fragments.push(fragment),
            Some(ignore) if prefix_contains(ignore, fragment) => {}
            Some(_) => pending.extend(split_prefix(fragment)?),
        }
    }
    fragments.sort();
    fragments.dedup();
    Some(fragments)
}

/// One device row's permitted mappings: each mapping is a PREFIX, so the ignored
/// subnets are subtracted from it instead of being compared with its network
/// address. A mapping longer than its family width is kept as configured; it was
/// never resolvable coverage. The first occurrence of a fragment wins, so two
/// overlapping mappings cannot emit one Map identity twice.
fn permitted_mappings(mappings: impl Iterator<Item = Prefix>, ignored: &[Prefix]) -> Vec<Prefix> {
    let mut permitted = Vec::new();
    let mut seen = HashSet::new();
    for mapping in mappings {
        let fragments =
            subtract_ignored_prefixes(mapping, ignored).unwrap_or_else(|| vec![mapping]);
        permitted.extend(fragments.into_iter().filter(|fragment| seen.insert(*fragment)));
    }
    permitted
}

/// What `ip_ranges.ignore_subnets` means to the folds (u8 REV-C1).
///
/// `prefixes` is the LENIENT parse — every entry that parsed, in configured
/// order — and it is empty for two worlds the slice alone cannot tell apart:
///
/// - the operator configured NO ignores: a catch-all fallback publishes its
///   whole rule, which is the documented contract, and
/// - the operator configured ignores and every one of them was unreadable: a
///   catch-all fallback that published its whole rule would expose exactly the
///   subnets the operator tried to exclude.
///
/// `configured` is that distinction, and [`Self::unusable`] is the one place it
/// is decided.
#[derive(Debug, Clone, Copy)]
struct IgnorePrefixes<'a> {
    /// The entries that parsed; the malformed ones were each reported once by
    /// `lqos_config` while it built this list.
    prefixes: &'a [Prefix],
    /// The operator configured at least one entry.
    configured: bool,
}

impl IgnorePrefixes<'_> {
    /// A list that was configured and yielded no usable prefix.
    ///
    /// Range fallback synthesis must DEFER here — no fallback row, no prefix
    /// `Map` — because the computed complement of a catch-all would be the
    /// catch-all itself: an unreadable exclusion must never become unfiltered
    /// `0.0.0.0/0` coverage. The ordinary device prune stays LENIENT: it
    /// subtracts what parsed, and a device row is never catch-all coverage, so
    /// the same typo only narrows that prune.
    fn unusable(&self) -> bool {
        self.configured && self.prefixes.is_empty()
    }
}

/// u5 A1 (REV-D1): fold each configured `[[dynamic_circuits.ranges]]` rule
/// into ONE ordinary DEVICE row over the rule's prefix, at the same seam the
/// RADIUS store fold uses — after the base import, before the operator override
/// fold and before the device loop's `ip_ranges.ignore_subnets` prune.
///
/// Packet-one coverage used to be a compiler-only leaf plus a prefix `Map`
/// injected at the transaction stage (`range_leaf` / `RangeRow`, retired in u5).
/// It is now an ordinary row, compiled by the normal leaf + `Map` path, so the
/// compiler, the broker diff and the data plane cannot tell a range fallback
/// from any other circuit. The prefix `Map` is still a longest-prefix entry, so
/// a promoted `/32` (and a provisioned more-specific row) shadows it.
///
/// IGNORE SUBTRACTION (u7, second Oracle consult): the row's mapping prefixes are
/// the rule MINUS `ip_ranges.ignore_subnets`, because explicit management
/// exclusions outrank catch-all coverage — a `/0` fallback must never classify
/// management traffic, and a promotion cap or warning cannot protect it. The rule
/// stays ONE circuit with ONE device carrying the computed prefix list at the
/// rule's rates; a rule whose complement is empty creates no row at all.
///
/// DEFERRAL (u8 REV-C1): the ignore list arrives here ALREADY PARSED and
/// lenient — `lqos_config` skips a malformed entry with its own once-per-entry
/// warning, and what parsed is subtracted, so one typo narrows the subtraction
/// instead of disabling it. Two cases still defer LOUDLY rather than publish
/// coverage: a prefix that cannot be represented after subtraction, and a
/// configured ignore list that yielded NO usable prefix at all (see
/// [`IgnorePrefixes::unusable`]). A list that was never configured is not a
/// deferral: with nothing to exclude, the rule's whole prefix is the intended
/// coverage.
///
/// WHY THE DEVICE SEAM: a circuit row appended after the ordinary passes
/// answered to none of them — `CircuitAdjustment::RemoveCircuit` and
/// `CircuitAdjustSpeed` for its id were silent no-ops, and the
/// `ip_ranges.ignore_subnets` prune never saw its prefix. A device row at the
/// RADIUS fold's seam inherits the whole operator control surface: persistent
/// overrides
/// (replace/append), the circuit/device adjustment fold, `ReparentCircuit`,
/// flat-bucket assignment, the ignored-subnet prune, the circuit-AQM post-pass
/// and the ordinary parent resolution. The prune below the seam re-subtracts the
/// same ignores from the fragments this fold already computed, which is a no-op
/// and keeps one ignore semantics for both rows.
///
/// Identity: `dynamic-range:<rule name>` for both the circuit and its single
/// device, the id the retired compiler emitted and the namespace the promotion
/// rows (`dynamic-range:<rule name>:<ip>`) extend, so a rule keeps its circuit
/// across publishes. A rule whose id collides with an assembled circuit is
/// REJECTED like a colliding RADIUS row.
///
/// Parent: blank `attach_to` is the dataplane root — the resolved parent id is
/// the empty sentinel the wire normalizes to `root`, and the store's synthetic
/// root node ("Root") is the row's display parent, so the circuit is attributed
/// to root instead of counting as an unmapped/missing-parent circuit. Non-blank
/// `attach_to` must resolve to an effective node; anything else DEFERS the rule
/// LOUDLY with no row at all, exactly the retired branch's behaviour.
///
/// Rates are the rule's four configured values; the leaf's AQM is pinned to an
/// explicit `none` because the retired unconfirmed leaf carried no AQM (Oracle
/// ruling 1: an ordinary leaf must not silently change intended rates/AQM). The
/// pin is an ordinary SQM token, so it stays subject to the ordinary
/// `DeviceAdjustSqm` / `CircuitAdjustAqm` passes. The gate is the retired
/// branch's: this fold is inert unless the dynamic feature AND unknown-IP
/// promotion are enabled.
///
/// Side effects: appends device rows to `shaped_devices` and records their
/// origin rows. Returns the LOUD warnings for rejected/deferred/ignored rules.
#[allow(clippy::too_many_arguments)]
fn fold_range_rules_into_devices(
    config: &Config,
    shaped_devices: &mut ConfigShapedDevices,
    exported_effective_nodes: &HashMap<String, String>,
    exported_effective_names: &HashMap<String, Option<String>>,
    exported_effective_aliases: &HashMap<String, (String, String)>,
    queue_aliases_by_id: &HashMap<String, (String, String)>,
    queue_aliases_by_name: &HashMap<String, (String, String)>,
    ignored: IgnorePrefixes<'_>,
    origin_rows: &mut lqos_utils::DynamicOriginRows,
) -> Vec<String> {
    let mut warnings = Vec::new();
    let rules = live_range_rules(config);
    if rules.is_empty() {
        return warnings;
    }
    // u8 A1 kept the LENIENT subtraction: a malformed entry is skipped by
    // `lqos_config` with its own warning, so the surviving prefixes are what
    // this fold subtracts and a typo narrows the prune.
    // u8 REV-C1 (repair 2) restores the ONE fail-closed case the lenient parse
    // cannot cover: a list that was configured and produced no usable prefix at
    // all. Subtracting nothing there would publish the rule whole — an unfiltered
    // `0.0.0.0/0` over the subnets the operator asked to exclude — so every
    // rule defers instead, loudly, exactly like the pre-u8 gate but without
    // its all-or-nothing trigger on a single bad line.
    let ignores_unusable = ignored.unusable();
    // REV-C1: an id is held only by a row that EXISTS (the assembled rows) or by
    // an ACCEPTED earlier rule. Nothing is reserved before the row is pushed, so a
    // rule that defers does not deny its id to a later valid rule — and the two
    // causes keep their own message.
    let assembled: HashSet<String> = shaped_devices
        .devices
        .iter()
        .map(|device| lqos_utils::normalize_circuit_id_key(&device.circuit_id))
        .collect();
    let mut claimed: HashSet<String> = HashSet::new();
    let mut rows: Vec<lqos_config::ShapedDevice> = Vec::new();
    for rule in rules {
        let rule_name = rule.name.trim();
        if rule_name.is_empty() {
            warnings.push(
                "a [[dynamic_circuits.ranges]] rule with an empty name was ignored".to_string(),
            );
            continue;
        }
        if ignores_unusable {
            warnings.push(format!(
                "dynamic_range_ignored_subnets_unusable: range '{rule_name}' is DEFERRED — \
                 ip_ranges.ignore_subnets is configured but no entry parses, so the rule's \
                 permitted coverage cannot be computed (no fallback row, no prefix Map)"
            ));
            continue;
        }
        let circuit_id = format!("dynamic-range:{rule_name}");
        let circuit_key = lqos_utils::normalize_circuit_id_key(&circuit_id);
        if assembled.contains(&circuit_key) {
            warnings.push(format!(
                "range '{rule_name}' collides with an assembled circuit id ('{circuit_id}'); \
                 rejected (a range fallback never shadows a provisioned or store circuit)"
            ));
            continue;
        }
        if claimed.contains(&circuit_key) {
            warnings.push(format!(
                "range '{rule_name}' is claimed by an earlier range rule (normalized id \
                 '{circuit_key}'); rejected (one fallback per rule identity)"
            ));
            continue;
        }
        // U4 parity: an ignored subnet never reaches `mapping_ops`. The rule's
        // mapping is its prefix MINUS every ignore, so a catch-all keeps only the
        // permitted fragments; an empty complement is a real answer (the rule is
        // entirely ignored) and it must be named rather than vanish.
        let Some(prefixes) = subtract_ignored_prefixes(
            (rule.ip_range.network_address(), u32::from(rule.ip_range.netmask())),
            ignored.prefixes,
        ) else {
            warnings.push(format!(
                "dynamic_range_ignored_subnet: range '{rule_name}' prefix {} cannot be represented \
                 after ip_ranges.ignore_subnets subtraction — the fallback is DEFERRED (no fallback \
                 row, no prefix Map)",
                rule.ip_range
            ));
            continue;
        };
        if prefixes.is_empty() {
            warnings.push(format!(
                "dynamic_range_ignored_subnet: range '{rule_name}' prefix {} is entirely inside \
                 ip_ranges.ignore_subnets — no fallback row is created (an ignored subnet never \
                 reaches mapping_ops)",
                rule.ip_range
            ));
            continue;
        }
        let attach = rule.attach_to.trim();
        let (parent_node_id, parent_node_name) = if attach.is_empty() {
            (String::new(), DATA_PLANE_ROOT_NAME.to_string())
        } else {
            match resolve_legacy_parent_from_effective_tree(
                attach,
                None,
                exported_effective_nodes,
                exported_effective_names,
                exported_effective_aliases,
                queue_aliases_by_id,
                queue_aliases_by_name,
            ) {
                Some((parent_node_id, parent_node_name)) if !parent_node_id.trim().is_empty() => {
                    (parent_node_id, parent_node_name)
                }
                _ => {
                    warnings.push(format!(
                        "dynamic_range_parent_unresolved: range '{rule_name}' attach_to {attach:?} \
                         matches no effective node — the range is DEFERRED (no fallback circuit, \
                         no prefix Map)"
                    ));
                    continue;
                }
            }
        };
        // The rule's permitted prefixes as the device row's mappings: each one is
        // a network address plus its own mask, so a v4 rule can never carry a
        // prefix above /32 (the bridge encodes v4 as 96 + prefix_len).
        let mut device = lqos_config::ShapedDevice {
            circuit_id: circuit_id.clone(),
            circuit_name: circuit_id.clone(),
            device_id: circuit_id.clone(),
            device_name: circuit_id.clone(),
            parent_node: parent_node_name,
            parent_node_id: optional_non_empty(&parent_node_id),
            sqm_override: Some("none".to_string()),
            download_min_mbps: rule.download_min_mbps,
            upload_min_mbps: rule.upload_min_mbps,
            download_max_mbps: rule.download_max_mbps,
            upload_max_mbps: rule.upload_max_mbps,
            ..lqos_config::ShapedDevice::default()
        };
        for (address, prefix_len) in prefixes {
            match address {
                IpAddr::V4(address) => device.ipv4.push((address, prefix_len)),
                IpAddr::V6(address) => device.ipv6.push((address, prefix_len)),
            }
        }
        // The origin marker is what keeps the fallback OUT of provisioned
        // coverage; the u3 supersede rule and the u4 promotion gate both read it.
        origin_rows.insert_row(&device.circuit_id, &device.device_id);
        claimed.insert(circuit_key);
        rows.push(device);
    }
    if !rows.is_empty() {
        let mut all = std::mem::take(&mut shaped_devices.devices);
        all.extend(rows);
        shaped_devices.replace_with_new_data(all);
    }
    warnings
}

/// K1 §7: a fallback-supplied identity (parent_node_id / anchor_node_id) is
/// valid only when it resolves to an existing effective node, an attachment
/// owner, or a queue alias — the same id classes the compile's anchor resolver
/// accepts. `None`/blank is the name-only working shape and always passes.
fn fallback_identity_resolves(
    id: Option<&str>,
    exported_effective_nodes: &HashMap<String, String>,
    attachment_owner_by_attachment_id: &HashMap<String, (String, String)>,
    queue_aliases_by_id: &HashMap<String, (String, String)>,
) -> bool {
    match id.map(str::trim).filter(|value| !value.is_empty()) {
        None => true,
        Some(id) => {
            exported_effective_nodes.contains_key(id)
                || attachment_owner_by_attachment_id.contains_key(id)
                || queue_aliases_by_id.contains_key(id)
        }
    }
}

/// Phase C S3b: applies one circuit's resolved match identity (from the
/// standard integration format, carried in the compiled-shaping map) to the
/// three circuit-input identity slots. Absent entries leave all three `None`
/// (the legacy and `ip`-mode behavior).
fn resolve_circuit_match_identity(
    index: &lqos_config::TopologyCircuitMatchIndex,
    circuit_id: &str,
) -> (
    Option<lqos_config::TunnelIdentity>,
    Option<u16>,
    Option<u32>,
) {
    match index.get(circuit_id).copied() {
        Some(lqos_config::TopologyCircuitMatchIdentity::Tunnel(identity)) => {
            (Some(identity), None, None)
        }
        Some(lqos_config::TopologyCircuitMatchIdentity::Vlan(vlan)) => (None, Some(vlan), None),
        Some(lqos_config::TopologyCircuitMatchIdentity::Mpls(label)) => (None, None, Some(label)),
        None => (None, None, None),
    }
}

fn build_shaping_inputs(
    config: &Config,
    artifacts: &EffectiveTopologyArtifacts,
) -> Result<Option<TopologyShapingInputsFile>> {
    build_shaping_inputs_with(config, artifacts, false)
}

fn build_shaping_inputs_with(
    config: &Config,
    artifacts: &EffectiveTopologyArtifacts,
    read_only: bool,
) -> Result<Option<TopologyShapingInputsFile>> {
    // T-arc: the validated tunnel-circuits snapshot is an additional,
    // device-less circuit source (spec v1.1 §4). It is read in BOTH ingress
    // modes and can stand alone when no ShapedDevices.csv exists.
    let tunnel_circuits = TunnelCircuitsFile::load_snapshot(config)
        .with_context(|| "Unable to load the tunnel_circuits snapshot while building shaping_inputs.json")?;
    // An accepted snapshot (even an explicit empty wipe) is a live source: it
    // must yield a transaction so removals emit DelTunnel. Only the total
    // absence of a snapshot lets shaping inputs collapse to None.
    let tunnel_snapshot_present = tunnel_circuits.is_some();
    // K1: a non-empty dynamic store is a live shaping source on its own.
    let dynamic_store_present =
        !lqos_config::DynamicCircuitsFile::load(config).circuits.is_empty();
    // u5: a configured range rule is live coverage on its own — a box with only
    // `[[dynamic_circuits.ranges]]` must still publish a fallback transaction.
    let range_rules_present = !live_range_rules(config).is_empty();

    // Phase C S4 (producer unification): ONE ingest contract. The runtime
    // materializes the compiled artifacts from the permanent manual pair when
    // an integration did not produce them, then reads the import only — the
    // integration-ingress "mode" no longer reaches this consumer.
    let imported_artifacts = match load_integration_shaping_artifacts(config)? {
        Some(artifacts) => Some(artifacts),
        None => {
            materialize_manual_topology_import(config)?;
            load_integration_shaping_artifacts(config)?
        }
    };
    let (mut shaped_devices, circuit_anchor_rows, circuit_matches) = match imported_artifacts {
        Some((shaped_devices, circuit_anchors, circuit_matches)) => {
            (shaped_devices, circuit_anchors, circuit_matches)
        }
        None
            if !tunnel_snapshot_present && !dynamic_store_present && !range_rules_present =>
        {
            return Ok(None);
        }
        None => (ConfigShapedDevices::default(), Vec::new(), Default::default()),
    };
    let effective_by_node = artifacts
        .effective
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let ui_by_node = artifacts
        .ui_state
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let mut exported_effective_nodes = HashMap::<String, String>::new();
    let mut exported_effective_names = HashMap::<String, Option<String>>::new();
    let mut exported_effective_aliases = HashMap::<String, (String, String)>::new();
    let mut ambiguous_exported_effective_names = HashSet::<String>::new();
    let attachment_owner_by_attachment_id = build_attachment_owner_map(&artifacts.ui_state);
    if let Some(effective_network) = artifacts.effective_network.as_ref() {
        collect_exported_effective_nodes(
            effective_network,
            &mut exported_effective_nodes,
            &mut exported_effective_names,
            &mut ambiguous_exported_effective_names,
        );
        collect_exported_effective_aliases(effective_network, &mut exported_effective_aliases);
    }
    let (queue_aliases_by_id, queue_aliases_by_name) = build_effective_queue_aliases(
        &artifacts.ui_state,
        &artifacts.effective,
        &exported_effective_nodes,
    );

    // K1 §1.1/§1.2: fold the RADIUS dynamic store at the DEVICE-ROW level,
    // AFTER the base import and BEFORE the operator override fold (so an
    // operator override on a dynamic circuit wins by existing precedence).
    let DynamicFoldOutcome {
        warnings: dynamic_fold_warnings,
        origin_rows,
    } = fold_dynamic_circuits_into_devices(
        config,
        &mut shaped_devices,
        &exported_effective_nodes,
        &exported_effective_names,
        &exported_effective_aliases,
        &attachment_owner_by_attachment_id,
        &queue_aliases_by_id,
        &queue_aliases_by_name,
    );

    // U4 (v2-parity Ignored Subnets; `src/src/configMigrator.py:39` defaults
    // `ignoreSubnets = ["192.168.0.0/16"]`): a device IP inside
    // `ip_ranges.ignore_subnets` must never reach the compiler's `mapping_ops`.
    // A3 (u7): the ignore list is a set of PREFIXES, so both consumers SUBTRACT
    // it — address containment is not prefix containment.
    // A1 (u8): the parse is LENIENT. `lqos_config` skips a malformed entry with
    // its own once-per-entry warning, so one typo narrows the prune instead of
    // disabling it (the pre-u8 all-or-nothing gate deferred every range rule when
    // a single line carried no `/<prefix length>`), and the publish thread can no
    // longer be taken down by a config line.
    // REV-C1: leniency is right for the DEVICE prune, which can only under-ignore
    // one device, and wrong for a CATCH-ALL fallback: with no usable prefix the
    // subtraction is a no-op and the fallback would publish `0.0.0.0/0` over the
    // subnets the operator excluded. `IgnorePrefixes` carries the two empty cases
    // apart (nothing configured vs configured-but-unreadable) so the range fold
    // can defer on the second while this prune stays lenient on both.
    //
    // The `Result` shape survives only for the lqosd consumers; there is no error
    // arm, and this publish path must NOT fail open if one ever appears. `expect`
    // keeps that posture (loud) until the follow-up drops the `Result` altogether.
    let ignored_prefixes = config
        .ip_ranges
        .ignored_network_table()
        .expect("the lenient ip_ranges parse has no error arm")
        .iter()
        .map(|(network, _)| (network.network_address(), u32::from(network.netmask())))
        .collect::<Vec<Prefix>>();
    let ignored = IgnorePrefixes {
        prefixes: &ignored_prefixes,
        configured: !config.ip_ranges.ignore_subnets.is_empty(),
    };

    // u5 A1 (REV-D1): the range fallback is a DEVICE row at the SAME seam, so the
    // ordinary passes below govern it exactly like every other circuit: the
    // override fold (removal / rate / reparent / SQM), the ignored-subnet prune,
    // flat-bucket assignment, parent resolution, and the circuit-AQM post-pass.
    let mut origin_rows = origin_rows;
    let range_fold_warnings = fold_range_rules_into_devices(
        config,
        &mut shaped_devices,
        &exported_effective_nodes,
        &exported_effective_names,
        &exported_effective_aliases,
        &queue_aliases_by_id,
        &queue_aliases_by_name,
        ignored,
        &mut origin_rows,
    );

    // Operator overrides fold into the shaped devices. Phase C S4: the ingest
    // contract is ONE, so the FULL override set (persistent devices +
    // adjustments — the UISP-regeneration-proof layer, E3 retarget
    // `docs/e3_derivation.md` rev 426) applies uniformly now that the manual
    // pair is compiled through the same import. The old legacy-DIY rate-only
    // scoping rail retired with the mode.
    let effective_overrides = load_runtime_shaping_overrides(config, read_only).with_context(
        || "Unable to load effective overrides while building shaping_inputs.json",
    )?;
    let (runtime_devices, replaced_origin_rows) =
        apply_runtime_shaped_device_overrides(shaped_devices.devices.clone(), &effective_overrides);
    shaped_devices.replace_with_new_data(runtime_devices);
    // A persistent override PROVISIONS the rows it supplies: an override row
    // REPLACES the row carrying its `device_id`, so that fold row is no longer
    // synthesized and leaves the origin record. A row the override APPENDS under
    // the same `circuit_id` keeps its own device id, so the fold's row stays
    // dynamic-origin — the marker is per ROW, exactly like the override fold.
    // Without this, an operator who pins a live dynamic circuit as a persistent
    // device would inherit the dynamic origin and lose the supersede rule.
    let dynamic_origin_rows = {
        let mut origins = origin_rows;
        origins.remove_replaced_rows(
            replaced_origin_rows
                .iter()
                .map(|(circuit_id, device_id)| (circuit_id.as_str(), device_id.as_str())),
        );
        origins
    };
    let flat_bucket_assignments = if runtime_flat_mode(config) {
        Some(build_flat_bucket_assignments(
            config,
            &shaped_devices.devices,
        ))
    } else {
        None
    };
    let circuit_anchors = circuit_anchor_rows
        .into_iter()
        .map(|anchor| (anchor.circuit_id.clone(), anchor))
        .collect::<HashMap<_, _>>();

    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    warnings.extend(dynamic_fold_warnings);
    warnings.extend(range_fold_warnings);
    let mut fallback_warning_summary = TopologyFallbackWarningSummary::default();
    let mut circuits = Vec::<TopologyShapingCircuitInput>::new();
    let mut circuits_by_id = HashMap::<String, usize>::new();

    // The ignored prefixes find no home in the device-less tunnel fold below; the
    // shaped-device rows are pruned here.
    for device in &shaped_devices.devices {
        // v2 `__pruneIgnoredCircuits` parity: keep only permitted addresses
        // (`isIntegrationOutputIpAllowed` = NOT ignored; the allow list is not
        // consulted). A3 (u7): the mapping is a PREFIX, so the ignores are
        // subtracted from it exactly like a range fallback's rule — an ignored
        // subnet no longer swallows a broader mapping whose network address it
        // happens to contain. A device that HAD addresses and loses them all is
        // dropped, and a circuit that never gains a permitted device is never
        // created below. A device with no addresses to begin with (v3's model
        // allows a mapping-less device) is left as-is.
        let had_addresses = !device.ipv4.is_empty() || !device.ipv6.is_empty();
        let filtered_ipv4 = permitted_mappings(
            device
                .ipv4
                .iter()
                .map(|(address, prefix_len)| (IpAddr::V4(*address), *prefix_len)),
            &ignored_prefixes,
        );
        let filtered_ipv6 = permitted_mappings(
            device
                .ipv6
                .iter()
                .map(|(address, prefix_len)| (IpAddr::V6(*address), *prefix_len)),
            &ignored_prefixes,
        );
        if had_addresses && filtered_ipv4.is_empty() && filtered_ipv6.is_empty() {
            continue;
        }
        let anchor_from_file = circuit_anchors.get(&device.circuit_id);
        let anchor_node_id = anchor_from_file
            .map(|anchor| anchor.anchor_node_id.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| optional_non_empty_owned(device.anchor_node_id.clone()));
        let mut anchor_node_name = anchor_from_file.and_then(|anchor| {
            anchor
                .anchor_node_name
                .as_ref()
                .and_then(|value| optional_non_empty(value))
        });
        let (
            mut effective_parent_node_id,
            mut effective_parent_node_name,
            mut effective_attachment_id,
            mut effective_attachment_name,
            mut resolution_source,
        ) = if let Some((bucket_id, bucket_name)) = flat_bucket_assignments
            .as_ref()
            .and_then(|assignments| assignments.get(&device.circuit_id))
        {
            (
                bucket_id.clone(),
                bucket_name.clone(),
                None,
                None,
                TopologyShapingResolutionSource::FlatBucket,
            )
        } else if let Some(anchor_id) = anchor_node_id.as_deref() {
            match resolve_effective_parent_from_anchor(
                anchor_id,
                &ui_by_node,
                &effective_by_node,
                &exported_effective_nodes,
                &attachment_owner_by_attachment_id,
                &queue_aliases_by_id,
            ) {
                Some((
                    resolved_parent_id,
                    resolved_parent_name,
                    resolved_attachment_id,
                    resolved_attachment_name,
                )) => {
                    if let Some(ui_node) = ui_by_node.get(anchor_id) {
                        anchor_node_name = Some(ui_node.node_name.clone());
                    }
                    (
                        resolved_parent_id,
                        resolved_parent_name,
                        resolved_attachment_id,
                        resolved_attachment_name,
                        TopologyShapingResolutionSource::TopologyAnchor,
                    )
                }
                None => match (
                    ui_by_node.get(anchor_id),
                    effective_by_node.get(anchor_id),
                    anchor_from_file,
                ) {
                    (Some(ui_node), Some(_effective_node), _) => {
                        anchor_node_name = Some(ui_node.node_name.clone());
                        fallback_warning_summary.record_unresolved_exported_anchor(
                            &device.circuit_id,
                            anchor_id,
                            &ui_node.node_name,
                        );
                        (
                            String::new(),
                            String::new(),
                            None,
                            None,
                            TopologyShapingResolutionSource::RuntimeFallback,
                        )
                    }
                    (None, None, Some(anchor)) => {
                        fallback_warning_summary.record_missing_anchor(
                            &device.circuit_id,
                            anchor_id,
                            anchor.anchor_node_name.as_deref(),
                        );
                        (
                            String::new(),
                            String::new(),
                            None,
                            None,
                            TopologyShapingResolutionSource::RuntimeFallback,
                        )
                    }
                    _ => {
                        fallback_warning_summary.record_missing_anchor(
                            &device.circuit_id,
                            anchor_id,
                            None,
                        );
                        (
                            String::new(),
                            String::new(),
                            None,
                            None,
                            TopologyShapingResolutionSource::RuntimeFallback,
                        )
                    }
                },
            }
        } else if let Some((resolved_parent_id, resolved_parent_name)) =
            resolve_legacy_parent_from_effective_tree(
                &device.parent_node,
                device.parent_node_id.as_deref(),
                &exported_effective_nodes,
                &exported_effective_names,
                &exported_effective_aliases,
                &queue_aliases_by_id,
                &queue_aliases_by_name,
            )
        {
            (
                resolved_parent_id,
                resolved_parent_name,
                None,
                None,
                TopologyShapingResolutionSource::LegacyParent,
            )
        } else {
            if optional_non_empty(&device.parent_node).is_some()
                || optional_non_empty_owned(device.parent_node_id.clone()).is_some()
            {
                fallback_warning_summary.record_missing_parent(
                    &device.circuit_id,
                    device.parent_node.trim(),
                    device.parent_node_id.clone().unwrap_or_default().trim(),
                );
            }
            (
                String::new(),
                String::new(),
                None,
                None,
                TopologyShapingResolutionSource::RuntimeFallback,
            )
        };

        if matches!(
            resolution_source,
            TopologyShapingResolutionSource::TopologyAnchor
                | TopologyShapingResolutionSource::LegacyParent
        ) && !effective_parent_is_exported(
            &effective_parent_node_id,
            &effective_parent_node_name,
            &exported_effective_nodes,
            &exported_effective_names,
        ) {
            fallback_warning_summary.record_non_exported_parent(
                &device.circuit_id,
                &effective_parent_node_name,
                &effective_parent_node_id,
            );
            effective_parent_node_id.clear();
            effective_parent_node_name.clear();
            effective_attachment_id = None;
            effective_attachment_name = None;
            resolution_source = TopologyShapingResolutionSource::RuntimeFallback;
        }

        let logical_parent_node_name = optional_non_empty(&device.parent_node);
        let logical_parent_node_id = optional_non_empty_owned(device.parent_node_id.clone());
        let (tunnel_identity, vlan_identity, mpls_identity) =
            resolve_circuit_match_identity(&circuit_matches, &device.circuit_id);
        let circuit_index = if let Some(index) = circuits_by_id.get(&device.circuit_id).copied() {
            let circuit = &mut circuits[index];
            if circuit.anchor_node_id != anchor_node_id {
                errors.push(format!(
                    "Circuit '{}' had multiple AnchorNodeID values while building shaping inputs.",
                    device.circuit_id
                ));
            }
            if circuit.effective_parent_node_id != effective_parent_node_id
                || circuit.effective_parent_node_name != effective_parent_node_name
            {
                errors.push(format!(
                    "Circuit '{}' resolved to multiple effective parents while building shaping_inputs.json.",
                    device.circuit_id
                ));
            }
            let conflicts = duplicate_circuit_shape_conflicts(circuit, device);
            if !conflicts.is_empty() {
                warnings.push(format!(
                    "Circuit '{}' had conflicting circuit-level fields across ShapedDevices.csv rows; using the first row's values and ignoring the duplicate row's circuit-level values for device '{}': {}.",
                    device.circuit_id,
                    device.device_id,
                    conflicts.join(", ")
                ));
            }
            index
        } else {
            let index = circuits.len();
            circuits_by_id.insert(device.circuit_id.clone(), index);
            circuits.push(TopologyShapingCircuitInput {
                circuit_id: device.circuit_id.clone(),
                circuit_name: device.circuit_name.clone(),
                anchor_node_id: anchor_node_id.clone(),
                anchor_node_name,
                logical_parent_node_name,
                logical_parent_node_id,
                effective_parent_node_name,
                effective_parent_node_id,
                effective_attachment_id,
                effective_attachment_name,
                resolution_source,
                download_min_mbps: device.download_min_mbps,
                upload_min_mbps: device.upload_min_mbps,
                download_max_mbps: device.download_max_mbps,
                upload_max_mbps: device.upload_max_mbps,
                comment: device.comment.clone(),
                sqm_override: device.sqm_override.clone(),
                tunnel_identity,
                vlan_identity,
                mpls_identity,
                devices: Vec::new(),
            });
            index
        };

        circuits[circuit_index]
            .devices
            .push(TopologyShapingDeviceInput {
                device_id: device.device_id.clone(),
                device_name: device.device_name.clone(),
                mac: device.mac.clone(),
                ipv4: filtered_ipv4.iter().map(prefix_to_string).collect(),
                ipv6: filtered_ipv6.iter().map(prefix_to_string).collect(),
                comment: device.comment.clone(),
                dynamic: dynamic_origin_rows.contains_row(&device.circuit_id, &device.device_id),
            });
    }

    // T-arc §4: fold the device-less tunnel circuits in. They are keyed by
    // (vni, vlan) at the bridge, so they carry no devices; the site node is
    // written to canonical state by the ingestion adapter and must already be
    // an exported effective node.
    if let Some(tunnel) = tunnel_circuits.as_ref() {
        for circuit in tunnel.to_shaping_circuits() {
            if circuits_by_id.contains_key(&circuit.circuit_id) {
                warnings.push(format!(
                    "Tunnel circuit '{}' collides with a ShapedDevices-derived circuit id; the device-derived circuit is kept and the tunnel entry is ignored.",
                    circuit.circuit_id
                ));
                continue;
            }
            if !exported_effective_nodes.contains_key(&circuit.effective_parent_node_id) {
                warnings.push(format!(
                    "Tunnel circuit '{}' references site '{}' which is not present in the effective topology; the circuit will not attach to a shaping node.",
                    circuit.circuit_id, circuit.effective_parent_node_id
                ));
            }
            circuits_by_id.insert(circuit.circuit_id.clone(), circuits.len());
            circuits.push(circuit);
        }
    }

    // K1b ph2: circuit-level SQM fallbacks (StormGuard `CircuitAdjustAqm`)
    // are a POST-pass on the assembled circuit rows, so a legacy device-level
    // `DeviceAdjustSqm` seed can never outvote them (§9.1-3).
    apply_circuit_aqm_overrides(&mut circuits, &effective_overrides, &mut warnings);

    circuits.sort_unstable_by(|left, right| left.circuit_id.cmp(&right.circuit_id));
    for circuit in &mut circuits {
        circuit
            .devices
            .sort_unstable_by(|left, right| left.device_id.cmp(&right.device_id));
    }
    fallback_warning_summary.append_to(&mut warnings);

    let unresolved_runtime_fallbacks = circuits
        .iter()
        .filter(|circuit| {
            circuit.effective_parent_node_id.trim().is_empty()
                && circuit.resolution_source == TopologyShapingResolutionSource::RuntimeFallback
        })
        .count();
    if unresolved_runtime_fallbacks > 0 {
        if config.shared_topology_compile_mode() == Some("flat") {
            warnings.push(format!(
                "Flat topology mode assigned {unresolved_runtime_fallbacks} circuit(s) to generated parent nodes during queue construction."
            ));
        } else {
            push_unresolved_runtime_fallback_summary(&mut warnings, unresolved_runtime_fallbacks);
        }
    }

    if !errors.is_empty() {
        let mut message = String::from(
            "Unable to build shaping_inputs.json due to runtime topology contract errors:",
        );
        for error in errors {
            message.push_str("\n- ");
            message.push_str(&error);
        }
        return Err(anyhow::anyhow!(message));
    }

    Ok(Some(TopologyShapingInputsFile {
        schema_version: 1,
        shaping_generation: String::new(),
        generated_unix: now_unix(),
        canonical_generated_unix: artifacts.effective.canonical_generated_unix,
        effective_generated_unix: artifacts.effective.generated_unix,
        warnings,
        circuits,
    }))
}

/// K1b ph2 (§9.1-3, §9.4): apply circuit-level AQM fallbacks as a POST-pass on
/// the assembled circuit rows. The stormguard `CircuitAdjustAqm` token wins over
/// any device-level `DeviceAdjustSqm` seed by construction (it runs after the
/// device-row expansion and first-device seeding). It runs in BOTH ingress modes
/// (the coordinator rev 640): the variant is a machine-authored runtime fallback persisted
/// in the layer file — unlike operator-authored `DeviceAdjustSqm`, whose
/// integration-only rule guards the legacy CSV round-trip. An absent circuit
/// WARNs loudly (§9.1-5).
pub(crate) fn apply_circuit_aqm_overrides(
    circuits: &mut [TopologyShapingCircuitInput],
    overrides: &lqos_overrides::OverrideFile,
    warnings: &mut Vec<String>,
) {
    let now_unix = now_unix_secs();
    for adjustment in overrides.circuit_adjustments() {
        let CircuitAdjustment::CircuitAdjustAqm {
            circuit_id,
            aqm_token,
            expires_at_unix,
        } = adjustment
        else {
            continue;
        };
        // §9.2 defense-in-depth: a lapsed entry must not apply even before the
        // periodic sweep drops it from the layer file.
        if expires_at_unix.is_some_and(|expires| expires <= now_unix) {
            continue;
        }
        let Some(circuit) = circuits
            .iter_mut()
            .find(|circuit| circuit.circuit_id == *circuit_id)
        else {
            warnings.push(format!(
                "stormguard_circuit_aqm_absent: circuit-level AQM fallback references circuit '{circuit_id}', which is absent from the assembled shaping rows; the entry was ignored."
            ));
            continue;
        };
        let token = aqm_token.trim();
        circuit.sqm_override = if token.is_empty() || token.eq_ignore_ascii_case("none") {
            None
        } else {
            Some(token.to_string())
        };
    }
}

/// The RATE-CLASS slice of the override fold (legacy-DIY mode):
/// CircuitAdjustSpeed / DeviceAdjustSpeed replace rates in place.
/// Persistent devices and removals are deliberately NOT folded here —
/// see the fold-site comment in `build_shaping_inputs`.
///
/// Returns the folded devices plus the `(circuit_id, device_id)` of every row a
/// persistent override REPLACED (the row identity the origin record subtracts;
/// a device id alone cannot identify it, since two dynamic circuits may share
/// one).
fn apply_runtime_shaped_device_overrides(
    base_devices: Vec<lqos_config::ShapedDevice>,
    overrides: &lqos_overrides::OverrideFile,
) -> (Vec<lqos_config::ShapedDevice>, Vec<(String, String)>) {
    let mut devices = base_devices;
    let mut replaced_rows: Vec<(String, String)> = Vec::new();
    for override_device in overrides.persistent_devices() {
        if let Some(existing_index) = devices
            .iter()
            .position(|device| device.device_id == override_device.device_id)
        {
            let replaced = &devices[existing_index];
            replaced_rows.push((replaced.circuit_id.clone(), replaced.device_id.clone()));
            devices[existing_index] = override_device.clone();
        } else {
            devices.push(override_device.clone());
        }
    }

    for adjustment in overrides.circuit_adjustments() {
        match adjustment {
            CircuitAdjustment::CircuitAdjustSpeed {
                circuit_id,
                min_download_bandwidth,
                max_download_bandwidth,
                min_upload_bandwidth,
                max_upload_bandwidth,
            } => {
                for device in devices
                    .iter_mut()
                    .filter(|device| device.circuit_id == *circuit_id)
                {
                    if let Some(value) = min_download_bandwidth {
                        device.download_min_mbps = *value;
                    }
                    if let Some(value) = max_download_bandwidth {
                        device.download_max_mbps = *value;
                    }
                    if let Some(value) = min_upload_bandwidth {
                        device.upload_min_mbps = *value;
                    }
                    if let Some(value) = max_upload_bandwidth {
                        device.upload_max_mbps = *value;
                    }
                }
            }
            CircuitAdjustment::DeviceAdjustSpeed {
                device_id,
                min_download_bandwidth,
                max_download_bandwidth,
                min_upload_bandwidth,
                max_upload_bandwidth,
            } => {
                for device in devices
                    .iter_mut()
                    .filter(|device| device.device_id == *device_id)
                {
                    if let Some(value) = min_download_bandwidth {
                        device.download_min_mbps = *value;
                    }
                    if let Some(value) = max_download_bandwidth {
                        device.download_max_mbps = *value;
                    }
                    if let Some(value) = min_upload_bandwidth {
                        device.upload_min_mbps = *value;
                    }
                    if let Some(value) = max_upload_bandwidth {
                        device.upload_max_mbps = *value;
                    }
                }
            }
            CircuitAdjustment::DeviceAdjustSqm {
                device_id,
                sqm_override,
            } => {
                for device in devices
                    .iter_mut()
                    .filter(|device| device.device_id == *device_id)
                {
                    device.sqm_override = sqm_override
                        .as_ref()
                        .map(|value| value.trim().to_string())
                        .filter(|value| !value.is_empty());
                }
            }
            // K1b ph2: circuit-level AQM is a POST-pass on the assembled
            // circuit rows (see `apply_circuit_aqm_overrides`), never a
            // device-row mutation.
            CircuitAdjustment::CircuitAdjustAqm { .. } => {}
            CircuitAdjustment::RemoveCircuit { circuit_id } => {
                devices.retain(|device| device.circuit_id != *circuit_id);
            }
            CircuitAdjustment::RemoveDevice { device_id } => {
                devices.retain(|device| device.device_id != *device_id);
            }
            CircuitAdjustment::ReparentCircuit {
                circuit_id,
                parent_node,
            } => {
                for device in devices
                    .iter_mut()
                    .filter(|device| device.circuit_id == *circuit_id)
                {
                    device.parent_node = parent_node.clone();
                    device.parent_node_id = None;
                    device.anchor_node_id = None;
                }
            }
        }
    }

    (devices, replaced_rows)
}
