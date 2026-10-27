// T-arc tunnel-circuits ingestion adapter (spec v1.1): the device-less
// tunnel circuits flow from the last-good snapshot into shaping_inputs, the
// site node (written to canonical state by the adapter) is kept, and the
// compiler emits a leaf + MapTunnel with no IP maps.

fn tunnel_test_config(prefix: &str) -> Config {
    let lqos_directory = unique_temp_dir(prefix);
    Config {
        lqos_directory: lqos_directory.to_string_lossy().to_string(),
        state_directory: None,
        ..Config::default()
    }
}

fn tunnel_canonical(sites: &[&str]) -> TopologyCanonicalStateFile {
    let mut canonical = TopologyCanonicalStateFile::default();
    for site in sites {
        canonical.nodes.push(TopologyCanonicalNode {
            node_id: format!("tunnel-circuits:site:{site}"),
            node_name: (*site).to_string(),
            node_kind: "site".to_string(),
            ..TopologyCanonicalNode::default()
        });
    }
    canonical
}

fn tunnel_entry(vni: u32, vlan: u16, subscriber: &str, site: &str) -> TunnelCircuitEntry {
    TunnelCircuitEntry {
        vni,
        vlan,
        subscriber_id: subscriber.to_string(),
        display_name: None,
        parent_site: site.to_string(),
        site_display_name: None,
        plan_down_mbps: 500,
        plan_up_mbps: 500,
    }
}

fn tunnel_artifacts(config: &Config, canonical: &TopologyCanonicalStateFile) -> EffectiveTopologyArtifacts {
    build_effective_topology_artifacts_from_canonical(
        config,
        canonical,
        &TopologyOverridesFile::default(),
        &TopologyAttachmentHealthStateFile::default(),
    )
    .expect("tunnel effective artifacts should build")
}

#[test]
fn tunnel_circuits_build_device_less_and_emit_map_tunnel() {
    let config = tunnel_test_config("lqos-topology-tunnel-happy");
    let canonical = tunnel_canonical(&["Riverside_MDU_7"]);
    TunnelCircuitsFile {
        schema_version: 1,
        allow_empty: false,
        generated_at: None,
        circuits: vec![tunnel_entry(1010, 64, "SUB-1", "Riverside_MDU_7")],
    }
    .save_snapshot(&config)
    .expect("tunnel snapshot should write");

    let artifacts = tunnel_artifacts(&config, &canonical);
    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("tunnel shaping inputs should build")
        .expect("tunnel shaping inputs should be present");

    let circuit = inputs
        .circuits
        .iter()
        .find(|circuit| circuit.circuit_id == "tunnel-circuits:circuit:SUB-1")
        .expect("the tunnel circuit must be present");
    assert!(circuit.devices.is_empty(), "tunnel circuits are device-less");
    assert_eq!(
        circuit.tunnel_identity,
        Some(TunnelIdentity { vni: 1010, vlan: 64 })
    );
    assert_eq!(
        circuit.effective_parent_node_id,
        "tunnel-circuits:site:Riverside_MDU_7"
    );

    let network = artifacts
        .effective_network
        .clone()
        .expect("effective network should exist");
    let tx = compile_dataplane_transaction(&network, &inputs);
    assert!(
        tx.inner_ops.iter().any(|op| matches!(
            op,
            DataplaneInnerOp::Upsert { node_id, .. }
                if node_id == "tunnel-circuits:site:Riverside_MDU_7"
        )),
        "the site node must be kept: {:?}",
        tx.inner_ops
    );
    assert!(
        tx.leaf_ops.iter().any(|op| matches!(
            op,
            DataplaneLeafOp::Upsert { circuit_id, .. }
                if circuit_id == "tunnel-circuits:circuit:SUB-1"
        )),
        "the tunnel circuit must emit a leaf: {:?}",
        tx.leaf_ops
    );
    assert!(
        tx.mapping_ops.iter().any(|op| matches!(
            op,
            DataplaneMappingOp::MapTunnel { vni: 1010, vlan: 64, circuit }
                if circuit == "tunnel-circuits:circuit:SUB-1"
        )),
        "the tunnel circuit must emit a MapTunnel: {:?}",
        tx.mapping_ops
    );
    assert!(
        tx.mapping_ops
            .iter()
            .all(|op| !matches!(op, DataplaneMappingOp::Map { .. })),
        "a device-less tunnel circuit must emit no IP maps: {:?}",
        tx.mapping_ops
    );
}

#[test]
fn tunnel_circuits_removal_drops_the_map_tunnel() {
    let config = tunnel_test_config("lqos-topology-tunnel-removal");
    let canonical = tunnel_canonical(&["Riverside_MDU_7"]);
    let artifacts = tunnel_artifacts(&config, &canonical);
    let network = artifacts
        .effective_network
        .clone()
        .expect("effective network should exist");

    TunnelCircuitsFile {
        schema_version: 1,
        allow_empty: false,
        generated_at: None,
        circuits: vec![tunnel_entry(1010, 64, "SUB-1", "Riverside_MDU_7")],
    }
    .save_snapshot(&config)
    .expect("initial tunnel snapshot should write");
    let before = build_shaping_inputs(&config, &artifacts)
        .expect("initial shaping inputs should build")
        .expect("initial shaping inputs should be present");
    assert!(
        compile_dataplane_transaction(&network, &before)
            .mapping_ops
            .iter()
            .any(|op| matches!(op, DataplaneMappingOp::MapTunnel { .. }))
    );

    // The entry is absent from the new snapshot (an explicit empty wipe).
    TunnelCircuitsFile {
        schema_version: 1,
        allow_empty: true,
        generated_at: None,
        circuits: vec![],
    }
    .save_snapshot(&config)
    .expect("empty tunnel snapshot should write");
    let after = build_shaping_inputs(&config, &artifacts)
        .expect("post-removal shaping inputs should build")
        .expect("post-removal shaping inputs should be present");
    assert!(
        !after
            .circuits
            .iter()
            .any(|circuit| circuit.circuit_id == "tunnel-circuits:circuit:SUB-1"),
        "the removed circuit must be gone from shaping inputs"
    );
    let tx = compile_dataplane_transaction(&network, &after);
    assert!(
        !tx.mapping_ops
            .iter()
            .any(|op| matches!(op, DataplaneMappingOp::MapTunnel { .. })),
        "the removed circuit must no longer emit a MapTunnel: {:?}",
        tx.mapping_ops
    );
    assert!(
        !tx.leaf_ops.iter().any(|op| matches!(
            op,
            DataplaneLeafOp::Upsert { circuit_id, .. }
                if circuit_id == "tunnel-circuits:circuit:SUB-1"
        )),
        "the removed circuit must no longer emit a leaf"
    );
}

#[test]
fn tunnel_circuits_alone_without_shaped_devices_csv() {
    let config = tunnel_test_config("lqos-topology-tunnel-no-csv");
    let canonical = tunnel_canonical(&["Lakeside_MDU_2"]);
    TunnelCircuitsFile {
        schema_version: 1,
        allow_empty: false,
        generated_at: None,
        circuits: vec![tunnel_entry(2020, 66, "SUB-9", "Lakeside_MDU_2")],
    }
    .save_snapshot(&config)
    .expect("tunnel snapshot should write");

    let artifacts = tunnel_artifacts(&config, &canonical);
    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("device-less tunnel shaping inputs should build")
        .expect("a tunnel snapshot alone must yield shaping inputs");
    assert_eq!(inputs.circuits.len(), 1);
    assert_eq!(
        inputs.circuits[0].circuit_id,
        "tunnel-circuits:circuit:SUB-9"
    );
}

// U4 (v2-parity Ignored Subnets): device IPs in `ip_ranges.ignore_subnets`
/// (`192.168.0.0/16` by v2 default) are pruned at the topology choke BEFORE the
/// compiler emits `mapping_ops`. The prune is scoped to SHAPED-DEVICE circuits
/// (before the tunnel fold) so device-less tunnel circuits survive. Requires:
/// (a) an all-ignored device's circuit is dropped; (b) a device keeping a WAN
/// `/32` survives minus the ignored address; (c) the tunnel circuit survives;
/// (4) ZERO duplicate mapping identities in the emitted transaction.
#[test]
fn ignored_subnets_prune_shaped_devices_and_keep_tunnel_circuits() {
    let config = tunnel_test_config("lqos-topology-ignored-subnets");
    let canonical = tunnel_canonical(&["Riverside_MDU_7"]);
    TunnelCircuitsFile {
        schema_version: 1,
        allow_empty: false,
        generated_at: None,
        circuits: vec![tunnel_entry(1010, 64, "SUB-1", "Riverside_MDU_7")],
    }
    .save_snapshot(&config)
    .expect("tunnel snapshot should write");

    fs::write(
        PathBuf::from(&config.lqos_directory).join("ShapedDevices.csv"),
        concat!(
            "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
            "\"circuit-wan\",\"WAN\",\"device-wan\",\"Device WAN\",\"Riverside_MDU_7\",\"tunnel-circuits:site:Riverside_MDU_7\",\"\",\"\",\"192.168.1.1/32,100.64.0.5/32\",\"\",\"10\",\"10\",\"100\",\"100\",\"\"\n",
            "\"circuit-ignored\",\"Ignored\",\"device-ignored\",\"Device Ignored\",\"Riverside_MDU_7\",\"tunnel-circuits:site:Riverside_MDU_7\",\"\",\"\",\"192.168.1.1/32\",\"\",\"10\",\"10\",\"100\",\"100\",\"\"\n",
        ),
    )
    .expect("ShapedDevices.csv should write");

    let artifacts = tunnel_artifacts(&config, &canonical);
    let inputs = build_shaping_inputs(&config, &artifacts)
        .expect("shaping inputs should build")
        .expect("shaping inputs should exist");

    // (c) the device-less tunnel circuit survives the prune.
    assert!(
        inputs
            .circuits
            .iter()
            .any(|c| c.circuit_id == "tunnel-circuits:circuit:SUB-1"),
        "the device-less tunnel circuit must survive the ignored-subnet prune"
    );
    // (a) the all-ignored circuit is dropped.
    assert!(
        !inputs
            .circuits
            .iter()
            .any(|c| c.circuit_id == "circuit-ignored"),
        "a circuit whose only device IPs are ignored must be pruned"
    );
    // (b) the WAN survivor keeps only the permitted address.
    let wan = inputs
        .circuits
        .iter()
        .find(|c| c.circuit_id == "circuit-wan")
        .expect("the WAN circuit must survive");
    assert_eq!(
        wan.devices[0].ipv4,
        vec!["100.64.0.5".to_string()],
        "only the non-ignored address remains"
    );

    // (4) ZERO duplicate mapping identities in the emitted transaction.
    let network = artifacts
        .effective_network
        .clone()
        .expect("effective network present");
    let tx = compile_dataplane_transaction(&network, &inputs);
    let mut seen: HashSet<String> = HashSet::new();
    for op in &tx.mapping_ops {
        let identity = match op {
            DataplaneMappingOp::Map { prefix_len, ip, .. } => format!("prefix:{prefix_len}:{ip}"),
            DataplaneMappingOp::MapTunnel { vni, vlan, .. } => format!("tunnel:{vni}:{vlan}"),
            DataplaneMappingOp::MapVlan { vlan, .. } => format!("vlan:{vlan}"),
            DataplaneMappingOp::MapMpls { label, .. } => format!("mpls:{label}"),
            DataplaneMappingOp::Del { .. }
            | DataplaneMappingOp::DelTunnel { .. }
            | DataplaneMappingOp::DelVlan { .. }
            | DataplaneMappingOp::DelMpls { .. }
            | DataplaneMappingOp::Clear => continue,
        };
        assert!(
            seen.insert(identity.clone()),
            "duplicate mapping identity in the emitted transaction: {identity}"
        );
    }
    assert!(
        !tx.mapping_ops.iter().any(|op| matches!(
            op,
            DataplaneMappingOp::Map { ip, .. } if ip.to_string() == "192.168.1.1"
        )),
        "the ignored 192.168.1.1 must not reach the compiler's mapping_ops"
    );
}
