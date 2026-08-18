fn topology_import_ingress_enabled(config: &Config) -> bool {
    config.uisp_integration.enable_uisp
        || config.splynx_integration.enable_splynx
        || config
            .netzur_integration
            .as_ref()
            .is_some_and(|integration| integration.enable_netzur)
        || config
            .visp_integration
            .as_ref()
            .is_some_and(|integration| integration.enable_visp)
        || config.powercode_integration.enable_powercode
        || config.sonar_integration.enable_sonar
        || config
            .wispgate_integration
            .as_ref()
            .is_some_and(|integration| integration.enable_wispgate)
}

fn runtime_flat_mode(config: &Config) -> bool {
    config.shared_topology_compile_mode() == Some("flat")
}

fn count_interface_tx_queues(interface_name: &str) -> Option<usize> {
    let path = Path::new("/sys/class/net")
        .join(interface_name)
        .join("queues");
    let entries = std::fs::read_dir(path).ok()?;
    let mut count = 0usize;
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if name.starts_with("tx-") {
            count += 1;
        }
    }
    Some(count)
}

fn runtime_flat_bucket_count(config: &Config) -> usize {
    let mut queues_available = config
        .queues
        .override_available_queues
        .map(|value| value as usize);
    if queues_available.is_none() {
        queues_available = if config.queues.dry_run {
            Some(16)
        } else {
            let internet_queues = count_interface_tx_queues(&config.internet_interface());
            let isp_queues = count_interface_tx_queues(&config.isp_interface());
            internet_queues
                .zip(isp_queues)
                .map(|(left, right)| left.min(right))
        };
    }

    let shaping_cpu_count = detect_shaping_cpus(config).shaping.len();
    let mut queue_count = queues_available.unwrap_or(shaping_cpu_count.max(1));
    if shaping_cpu_count > 0 {
        queue_count = queue_count.min(shaping_cpu_count);
    }
    if config.on_a_stick_mode() {
        queue_count = (queue_count / 2).max(1);
    }
    queue_count.max(1)
}

fn runtime_flat_bucket_name(index: usize) -> String {
    format!("Generated_PN_{}", index + 1)
}

fn runtime_flat_bucket_id(index: usize) -> String {
    format!("libreqos:generated:flat:bucket:{index}")
}

fn current_flat_bucket_names(config: &Config) -> Option<BTreeSet<String>> {
    let Value::Object(current_network) = read_json_value(&topology_effective_network_path(config))?
    else {
        return None;
    };
    if current_network.is_empty() {
        return None;
    }

    let mut bucket_names = BTreeSet::new();
    for index in 0..current_network.len() {
        let bucket_name = runtime_flat_bucket_name(index);
        let expected_id = runtime_flat_bucket_id(index);
        let node = current_network.get(&bucket_name)?.as_object()?;
        if node.get("id").and_then(Value::as_str) != Some(expected_id.as_str()) {
            return None;
        }
        if !node
            .get("children")
            .and_then(Value::as_object)
            .is_some_and(|children| children.is_empty())
        {
            return None;
        }
        bucket_names.insert(bucket_name);
    }
    (bucket_names.len() == current_network.len()).then_some(bucket_names)
}

fn previous_flat_bucket_assignments(
    config: &Config,
    bucket_names: &[String],
) -> BTreeMap<String, String> {
    let expected_bucket_names = bucket_names.iter().cloned().collect::<BTreeSet<_>>();
    if current_flat_bucket_names(config).as_ref() != Some(&expected_bucket_names) {
        return BTreeMap::new();
    }

    let bucket_ids_by_name = bucket_names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.as_str(), runtime_flat_bucket_id(index)))
        .collect::<HashMap<_, _>>();
    let Ok(previous_inputs) = TopologyShapingInputsFile::load(config) else {
        return BTreeMap::new();
    };

    previous_inputs
        .circuits
        .into_iter()
        .filter_map(|circuit| {
            if circuit.resolution_source != TopologyShapingResolutionSource::FlatBucket {
                return None;
            }
            let expected_bucket_id =
                bucket_ids_by_name.get(circuit.effective_parent_node_name.as_str())?;
            (circuit.effective_parent_node_id == *expected_bucket_id)
                .then_some((circuit.circuit_id, circuit.effective_parent_node_name))
        })
        .collect()
}

fn runtime_flat_bucket_network(config: &Config) -> Value {
    let mut root = Map::new();
    for index in 0..runtime_flat_bucket_count(config) {
        let mut node = Map::new();
        node.insert("children".to_string(), Value::Object(Map::new()));
        node.insert(
            "downloadBandwidthMbps".to_string(),
            Value::Number(config.queues.generated_pn_download_mbps.into()),
        );
        node.insert(
            "uploadBandwidthMbps".to_string(),
            Value::Number(config.queues.generated_pn_upload_mbps.into()),
        );
        node.insert(
            "id".to_string(),
            Value::String(runtime_flat_bucket_id(index)),
        );
        node.insert(
            "name".to_string(),
            Value::String(runtime_flat_bucket_name(index)),
        );
        node.insert("type".to_string(), Value::String("Site".to_string()));
        root.insert(runtime_flat_bucket_name(index), Value::Object(node));
    }
    Value::Object(root)
}

fn build_flat_bucket_assignments(
    config: &Config,
    devices: &[lqos_config::ShapedDevice],
) -> HashMap<String, (String, String)> {
    let bucket_count = runtime_flat_bucket_count(config);
    if bucket_count == 0 {
        return HashMap::new();
    }

    let bucket_names = (0..bucket_count)
        .map(runtime_flat_bucket_name)
        .collect::<Vec<_>>();
    let bucket_ids_by_name = bucket_names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), runtime_flat_bucket_id(index)))
        .collect::<HashMap<_, _>>();

    let mut item_weights = BTreeMap::<String, f64>::new();
    for device in devices {
        let weight = f64::from(device.download_max_mbps.max(0.0) + device.upload_max_mbps.max(0.0));
        let sanitized = if weight.is_finite() && weight > 0.0 {
            weight
        } else {
            1.0
        };
        item_weights
            .entry(device.circuit_id.clone())
            .and_modify(|current| {
                if sanitized > *current {
                    *current = sanitized;
                }
            })
            .or_insert(sanitized);
    }

    let items = item_weights
        .into_iter()
        .map(|(id, weight)| TopLevelPlannerItem { id, weight })
        .collect::<Vec<_>>();
    let previous_assignments = if config.queues.use_binpacking {
        previous_flat_bucket_assignments(config, &bucket_names)
    } else {
        BTreeMap::new()
    };
    let planner_mode = if config.queues.use_binpacking {
        TopLevelPlannerMode::StableGreedy
    } else {
        TopLevelPlannerMode::RoundRobin
    };
    let planner = plan_top_level_assignments(
        &items,
        &bucket_names,
        &previous_assignments,
        &BTreeMap::new(),
        lqos_utils::unix_time::unix_now()
            .map(|timestamp| timestamp as f64)
            .unwrap_or(0.0),
        &TopLevelPlannerParams {
            mode: planner_mode,
            hysteresis_threshold: 0.03,
            cooldown_seconds: 0.0,
            move_budget_per_run: 1,
        },
    );

    planner
        .assignment
        .into_iter()
        .filter_map(|(circuit_id, bucket_name)| {
            bucket_ids_by_name
                .get(&bucket_name)
                .map(|bucket_id| (circuit_id, (bucket_id.clone(), bucket_name)))
        })
        .collect()
}
