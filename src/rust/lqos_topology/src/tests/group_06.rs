fn flat_test_config(prefix: &str, queue_count: u32) -> Config {
    let lqos_directory = unique_temp_dir(prefix);
    let mut config = Config {
        lqos_directory: lqos_directory.to_string_lossy().to_string(),
        state_directory: None,
        ..Config::default()
    };
    config.topology.compile_mode = "flat".to_string();
    config.queues.override_available_queues = Some(queue_count);
    config.queues.use_binpacking = true;
    config
}

fn write_flat_test_devices(config: &Config, circuits: &[(&str, f32)]) {
    let mut csv = String::from(
        "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Parent Node ID,Anchor Node ID,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment\n",
    );
    for (index, (circuit_id, rate)) in circuits.iter().enumerate() {
        csv.push_str(&format!(
            "\"{circuit_id}\",\"{circuit_id}\",\"device-{index}\",\"Device {index}\",\"\",\"\",\"\",\"02:00:00:00:00:{index:02x}\",\"192.0.2.{}/32\",\"\",\"10\",\"10\",\"{rate}\",\"{rate}\",\"\"\n",
            index + 1,
        ));
    }
    fs::write(
        PathBuf::from(&config.lqos_directory).join("ShapedDevices.csv"),
        csv,
    )
    .expect("flat ShapedDevices.csv should write");
}

fn flat_test_artifacts(config: &Config) -> EffectiveTopologyArtifacts {
    let canonical = TopologyCanonicalStateFile::from_legacy_network_json(&json!({}));
    build_effective_topology_artifacts_from_canonical(
        config,
        &canonical,
        &TopologyOverridesFile::default(),
        &TopologyAttachmentHealthStateFile::default(),
    )
    .expect("flat mode effective artifacts should build")
}

fn flat_assignment(inputs: &TopologyShapingInputsFile) -> BTreeMap<String, String> {
    inputs
        .circuits
        .iter()
        .map(|circuit| {
            (
                circuit.circuit_id.clone(),
                circuit.effective_parent_node_name.clone(),
            )
        })
        .collect()
}

fn changed_existing_assignments(
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> usize {
    before
        .iter()
        .filter(|(circuit_id, bucket)| {
            after
                .get(*circuit_id)
                .is_some_and(|next_bucket| next_bucket != *bucket)
        })
        .count()
}

#[test]
fn flat_binpacking_reuses_assignments_for_identical_input() {
    let config = flat_test_config("lqos-topology-flat-stable-identical", 2);
    write_flat_test_devices(
        &config,
        &[
            ("circuit-a", 100.0),
            ("circuit-b", 80.0),
            ("circuit-c", 60.0),
            ("circuit-d", 40.0),
        ],
    );
    let artifacts = flat_test_artifacts(&config);
    publish_effective_topology_artifacts(&config, &artifacts, "flat-identical")
        .expect("initial flat topology should publish");
    let before =
        TopologyShapingInputsFile::load(&config).expect("published shaping inputs should load");

    let after = build_shaping_inputs(&config, &artifacts)
        .expect("repeated flat shaping inputs should build")
        .expect("repeated flat shaping inputs should be present");

    assert_eq!(flat_assignment(&before), flat_assignment(&after));
}

#[test]
fn flat_binpacking_bounds_existing_moves_when_circuits_change() {
    let config = flat_test_config("lqos-topology-flat-stable-membership", 2);
    let initial_circuits = [
        ("circuit-b", 100.0),
        ("circuit-c", 80.0),
        ("circuit-d", 60.0),
        ("circuit-e", 40.0),
    ];
    write_flat_test_devices(&config, &initial_circuits);
    let artifacts = flat_test_artifacts(&config);
    publish_effective_topology_artifacts(&config, &artifacts, "flat-before-add")
        .expect("initial flat topology should publish");
    let before_add = flat_assignment(
        &TopologyShapingInputsFile::load(&config).expect("initial shaping inputs should load"),
    );

    let with_added_circuit = [
        ("circuit-a", 120.0),
        ("circuit-b", 100.0),
        ("circuit-c", 80.0),
        ("circuit-d", 60.0),
        ("circuit-e", 40.0),
    ];
    write_flat_test_devices(&config, &with_added_circuit);
    let after_add_inputs = build_shaping_inputs(&config, &artifacts)
        .expect("flat shaping inputs with an added circuit should build")
        .expect("flat shaping inputs with an added circuit should be present");
    let after_add = flat_assignment(&after_add_inputs);
    assert!(changed_existing_assignments(&before_add, &after_add) <= 1);

    publish_effective_topology_artifacts(&config, &artifacts, "flat-before-remove")
        .expect("flat topology with the added circuit should publish");
    let after_add_published = flat_assignment(
        &TopologyShapingInputsFile::load(&config)
            .expect("added-circuit shaping inputs should load"),
    );
    write_flat_test_devices(
        &config,
        &[
            ("circuit-a", 120.0),
            ("circuit-b", 100.0),
            ("circuit-d", 60.0),
            ("circuit-e", 40.0),
        ],
    );
    let after_remove = flat_assignment(
        &build_shaping_inputs(&config, &artifacts)
            .expect("flat shaping inputs with a removed circuit should build")
            .expect("flat shaping inputs with a removed circuit should be present"),
    );

    assert!(changed_existing_assignments(&after_add_published, &after_remove) <= 1);
}

#[test]
fn flat_binpacking_discards_invalid_or_wrong_queue_count_history() {
    let mut config = flat_test_config("lqos-topology-flat-invalid-history", 2);
    let circuits = [
        ("circuit-a", 100.0),
        ("circuit-b", 80.0),
        ("circuit-c", 60.0),
        ("circuit-d", 40.0),
    ];
    write_flat_test_devices(&config, &circuits);
    let artifacts = flat_test_artifacts(&config);
    let cold_assignment = flat_assignment(
        &build_shaping_inputs(&config, &artifacts)
            .expect("cold flat shaping inputs should build")
            .expect("cold flat shaping inputs should be present"),
    );
    publish_effective_topology_artifacts(&config, &artifacts, "flat-invalid-history")
        .expect("initial flat topology should publish");

    let mut invalid_history =
        TopologyShapingInputsFile::load(&config).expect("published shaping inputs should load");
    for circuit in &mut invalid_history.circuits {
        let (bucket_name, bucket_id) = if circuit.effective_parent_node_name == "Generated_PN_1" {
            ("Generated_PN_2", "libreqos:generated:flat:bucket:1")
        } else {
            ("Generated_PN_1", "libreqos:generated:flat:bucket:0")
        };
        circuit.effective_parent_node_name = bucket_name.to_string();
        circuit.effective_parent_node_id = bucket_id.to_string();
    }
    invalid_history
        .circuits
        .iter_mut()
        .find(|circuit| circuit.circuit_id == "circuit-a")
        .expect("circuit-a should exist")
        .effective_parent_node_id = "wrong-bucket-id".to_string();
    invalid_history
        .circuits
        .iter_mut()
        .find(|circuit| circuit.circuit_id == "circuit-b")
        .expect("circuit-b should exist")
        .resolution_source = TopologyShapingResolutionSource::RuntimeFallback;
    let deliberately_retained_assignment = flat_assignment(&invalid_history);
    invalid_history
        .save(&config)
        .expect("invalid shaping history should save");

    let rebuilt_assignment = flat_assignment(
        &build_shaping_inputs(&config, &artifacts)
            .expect("flat shaping inputs should rebuild from invalid history")
            .expect("rebuilt flat shaping inputs should be present"),
    );
    assert_eq!(
        rebuilt_assignment["circuit-a"],
        cold_assignment["circuit-a"]
    );
    assert_eq!(
        rebuilt_assignment["circuit-b"],
        cold_assignment["circuit-b"]
    );
    assert_eq!(
        rebuilt_assignment["circuit-c"],
        deliberately_retained_assignment["circuit-c"]
    );
    assert_eq!(
        rebuilt_assignment["circuit-d"],
        deliberately_retained_assignment["circuit-d"]
    );

    config.queues.override_available_queues = Some(3);
    let three_bucket_artifacts = flat_test_artifacts(&config);
    let after_queue_change = flat_assignment(
        &build_shaping_inputs(&config, &three_bucket_artifacts)
            .expect("flat shaping inputs should rebuild after queue-count change")
            .expect("queue-count rebuild should produce shaping inputs"),
    );

    let cold_three_bucket_config = flat_test_config("lqos-topology-flat-cold-three-buckets", 3);
    write_flat_test_devices(&cold_three_bucket_config, &circuits);
    let cold_three_bucket_artifacts = flat_test_artifacts(&cold_three_bucket_config);
    let cold_three_bucket_assignment = flat_assignment(
        &build_shaping_inputs(&cold_three_bucket_config, &cold_three_bucket_artifacts)
            .expect("cold three-bucket shaping inputs should build")
            .expect("cold three-bucket shaping inputs should be present"),
    );

    assert_eq!(after_queue_change, cold_three_bucket_assignment);
}
