//! Python bindings for LibreQoS configuration, queue orchestration, and daemon queries.

#![allow(non_local_definitions)] // Temporary: rewrite required for much of this, for newer PyO3.
#![allow(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]
use csv::ReaderBuilder;
use lqos_bus::{
    BlackboardSystem, BusRequest, BusResponse, IntegrationErrorClass, SchedulerIntegrationStatus,
    SchedulerProgressReport, SchedulerRun, UrgentSeverity, UrgentSource,
};
use lqos_topology_compile::{
    CompiledTopologyBundle, ImportedTopologyBundle, TopologyCompileMode,
    TopologyCompiledShapingFile, TopologyImportFile, compile_topology, parse_libreqos_network,
};
use lqos_utils::rustls::ensure_rustls_crypto_provider;
use nix::libc::getpid;
use pyo3::exceptions::PyOSError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use std::fmt::Write as _;
use std::{
    fs::{File, read_to_string, remove_file},
    io::Write,
    path::{Path, PathBuf},
};
mod blocking;
use anyhow::Result;
use blocking::{run_query, run_query_wait_for_bus};
use sysinfo::System;
mod device_weights;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

type PyObject = Py<PyAny>;

// ===== Planner CBOR I/O =====

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct PlannerSiteEntry {
    cpu: i64,
    major: i64,
    minor: i64,
    #[serde(default)]
    insertion_order: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct PlannerCircuitEntry {
    cpu: i64,
    major: i64,
    minor: i64,
    #[serde(default)]
    parent_site: String,
    #[serde(default)]
    sqm: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct PlannerStateSerde {
    #[serde(default = "default_algo_version")]
    algo_version: String,
    #[serde(default)]
    updated_at: f64,
    #[serde(default, rename = "queuesAvailable")]
    queues_available: i64,
    #[serde(default)]
    on_a_stick: bool,
    #[serde(default)]
    site_count: i64,
    #[serde(default)]
    site_names: Vec<i64>,
    #[serde(default)]
    site_map: BTreeMap<i64, PlannerSiteEntry>,
    #[serde(default)]
    circuit_map: BTreeMap<i64, PlannerCircuitEntry>,
}

fn default_algo_version() -> String {
    "v1".to_string()
}

fn load_shaped_devices_from_csv_payload(
    payload: &str,
) -> anyhow::Result<lqos_config::ConfigShapedDevices> {
    let mut reader = ReaderBuilder::new()
        .comment(Some(b'#'))
        .trim(csv::Trim::All)
        .from_reader(payload.as_bytes());
    let headers = reader.headers()?.clone();
    let mut devices = Vec::new();
    for record in reader.records() {
        let record = record?;
        let device = lqos_config::ShapedDevice::from_csv(&record, Some(&headers))?;
        devices.push(device);
    }
    let mut shaped_devices = lqos_config::ConfigShapedDevices::default();
    shaped_devices.replace_with_new_data(devices);
    Ok(shaped_devices)
}

fn to_i64_any(v: &pyo3::Bound<'_, pyo3::types::PyAny>) -> Option<i64> {
    if let Ok(n) = v.extract::<i64>() {
        return Some(n);
    }
    if let Ok(s) = v.extract::<String>() {
        return s.parse::<i64>().ok();
    }
    None
}

fn get_string(d: &pyo3::Bound<'_, pyo3::types::PyDict>, key: &str, default: String) -> String {
    match d.get_item(key) {
        Ok(Some(v)) => v.extract::<String>().unwrap_or(default),
        _ => default,
    }
}

fn get_f64(d: &pyo3::Bound<'_, pyo3::types::PyDict>, key: &str, default: f64) -> f64 {
    match d.get_item(key) {
        Ok(Some(v)) => v.extract::<f64>().unwrap_or(default),
        _ => default,
    }
}

fn get_i64(d: &pyo3::Bound<'_, pyo3::types::PyDict>, key: &str, default: i64) -> i64 {
    match d.get_item(key) {
        Ok(Some(v)) => v.extract::<i64>().unwrap_or(default),
        _ => default,
    }
}

fn get_bool(d: &pyo3::Bound<'_, pyo3::types::PyDict>, key: &str, default: bool) -> bool {
    match d.get_item(key) {
        Ok(Some(v)) => v.extract::<bool>().unwrap_or(default),
        _ => default,
    }
}

#[pyfunction(
    signature = (
        items,
        queues_available,
        prev_assign=None,
        last_change_ts=None,
        now_ts=None,
        mode=None,
        move_budget_per_run=None,
        cooldown_seconds=None,
        hysteresis_threshold=None
    )
)]
#[allow(clippy::too_many_arguments)]
fn plan_top_level_cpu_bins(
    py: Python,
    items: PyObject,
    queues_available: usize,
    prev_assign: Option<PyObject>,
    last_change_ts: Option<PyObject>,
    now_ts: Option<f64>,
    mode: Option<String>,
    move_budget_per_run: Option<usize>,
    cooldown_seconds: Option<f64>,
    hysteresis_threshold: Option<f64>,
) -> PyResult<PyObject> {
    let items_any = items.bind(py);
    let items_list = items_any.cast::<PyList>()?;
    let planner_items: Vec<lqos_config::TopLevelPlannerItem> = items_list
        .iter()
        .map(|item| -> PyResult<lqos_config::TopLevelPlannerItem> {
            let dict = item.cast::<PyDict>()?;
            let id = get_string(dict, "id", String::new());
            let weight = get_f64(dict, "weight", 1.0);
            Ok(lqos_config::TopLevelPlannerItem { id, weight })
        })
        .collect::<PyResult<Vec<_>>>()?;

    let prev_assign_map = match prev_assign {
        Some(obj) => {
            let any = obj.bind(py);
            let dict = any.cast::<PyDict>()?;
            dict.iter()
                .filter_map(|(k, v)| {
                    Some((k.extract::<String>().ok()?, v.extract::<String>().ok()?))
                })
                .collect::<BTreeMap<_, _>>()
        }
        None => BTreeMap::new(),
    };
    let last_change_map = match last_change_ts {
        Some(obj) => {
            let any = obj.bind(py);
            let dict = any.cast::<PyDict>()?;
            dict.iter()
                .filter_map(|(k, v)| Some((k.extract::<String>().ok()?, v.extract::<f64>().ok()?)))
                .collect::<BTreeMap<_, _>>()
        }
        None => BTreeMap::new(),
    };

    let bins: Vec<String> = (0..queues_available)
        .map(|idx| format!("CpueQueue{idx}"))
        .collect();
    let planner_mode = match mode.as_deref() {
        Some("round_robin") => lqos_config::TopLevelPlannerMode::RoundRobin,
        Some("greedy") => lqos_config::TopLevelPlannerMode::Greedy,
        _ => lqos_config::TopLevelPlannerMode::StableGreedy,
    };
    let params = lqos_config::TopLevelPlannerParams {
        mode: planner_mode,
        hysteresis_threshold: hysteresis_threshold.unwrap_or(0.03),
        cooldown_seconds: cooldown_seconds.unwrap_or(3600.0),
        move_budget_per_run: move_budget_per_run.unwrap_or(1),
    };
    let result = lqos_config::plan_top_level_assignments(
        &planner_items,
        &bins,
        &prev_assign_map,
        &last_change_map,
        now_ts.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0)
        }),
        &params,
    );

    let out = PyDict::new(py);
    let assignment = PyDict::new(py);
    for (item, bin) in result.assignment {
        assignment.set_item(item, bin)?;
    }
    out.set_item("assignment", assignment)?;
    out.set_item("changed", result.changed)?;
    out.set_item("planner_used", result.planner_used)?;
    Ok(out.into())
}

#[pyfunction(
    signature = (
        sites,
        circuit_groups,
        site_state=None,
        circuit_state=None,
        stick_offset=0,
        circuit_padding=8
    )
)]
fn plan_class_identities(
    py: Python,
    sites: PyObject,
    circuit_groups: PyObject,
    site_state: Option<PyObject>,
    circuit_state: Option<PyObject>,
    stick_offset: u16,
    circuit_padding: u32,
) -> PyResult<PyObject> {
    let sites_list = sites.bind(py).cast::<PyList>()?;
    let site_inputs: Vec<lqos_config::SiteIdentityInput> = sites_list
        .iter()
        .map(|item| -> PyResult<lqos_config::SiteIdentityInput> {
            let dict = item.cast::<PyDict>()?;
            Ok(lqos_config::SiteIdentityInput {
                site_key: get_string(dict, "site_key", String::new()),
                parent_path: get_string(dict, "parent_path", String::new()),
                queue: get_i64(dict, "queue", 0).max(0) as u32,
                has_children: get_bool(dict, "has_children", false),
            })
        })
        .collect::<PyResult<Vec<_>>>()?;

    let groups_list = circuit_groups.bind(py).cast::<PyList>()?;
    let circuit_group_inputs: Vec<lqos_config::CircuitIdentityGroupInput> = groups_list
        .iter()
        .map(|item| -> PyResult<lqos_config::CircuitIdentityGroupInput> {
            let dict = item.cast::<PyDict>()?;
            let ids = match dict.get_item("circuit_ids")? {
                Some(value) => value
                    .cast::<PyList>()?
                    .iter()
                    .filter_map(|entry| entry.extract::<String>().ok())
                    .collect::<Vec<_>>(),
                None => Vec::new(),
            };
            Ok(lqos_config::CircuitIdentityGroupInput {
                parent_node: get_string(dict, "parent_node", String::new()),
                queue: get_i64(dict, "queue", 0).max(0) as u32,
                circuit_ids: ids,
            })
        })
        .collect::<PyResult<Vec<_>>>()?;

    let previous_sites = match site_state {
        Some(obj) => {
            let dict = obj.bind(py).cast::<PyDict>()?;
            dict.iter()
                .filter_map(|(k, v)| {
                    let key = k.extract::<String>().ok()?;
                    let entry = v.cast::<PyDict>().ok()?;
                    Some((
                        key,
                        lqos_config::PlannerSiteIdentityState {
                            class_minor: get_i64(entry, "class_minor", 0).max(0) as u16,
                            queue: get_i64(entry, "queue", 0).max(0) as u32,
                            parent_path: get_string(entry, "parent_path", String::new()),
                            class_major: get_i64(entry, "class_major", 0).max(0) as u16,
                            up_class_major: get_i64(entry, "up_class_major", 0).max(0) as u16,
                        },
                    ))
                })
                .collect::<BTreeMap<_, _>>()
        }
        None => BTreeMap::new(),
    };

    let previous_circuits = match circuit_state {
        Some(obj) => {
            let dict = obj.bind(py).cast::<PyDict>()?;
            dict.iter()
                .filter_map(|(k, v)| {
                    let key = k.extract::<String>().ok()?;
                    let entry = v.cast::<PyDict>().ok()?;
                    Some((
                        key,
                        lqos_config::PlannerCircuitIdentityState {
                            class_minor: get_i64(entry, "class_minor", 0).max(0) as u16,
                            queue: get_i64(entry, "queue", 0).max(0) as u32,
                            parent_node: get_string(entry, "parent_node", String::new()),
                            class_major: get_i64(entry, "class_major", 0).max(0) as u16,
                            up_class_major: get_i64(entry, "up_class_major", 0).max(0) as u16,
                        },
                    ))
                })
                .collect::<BTreeMap<_, _>>()
        }
        None => BTreeMap::new(),
    };

    let result = lqos_config::plan_class_identities(
        &site_inputs,
        &circuit_group_inputs,
        &previous_sites,
        &previous_circuits,
        stick_offset,
        circuit_padding,
    );

    let out = PyDict::new(py);
    let sites_out = PyList::empty(py);
    for site in result.sites {
        let d = PyDict::new(py);
        d.set_item("site_key", site.site_key)?;
        d.set_item("queue", site.queue)?;
        d.set_item("class_minor", site.class_minor)?;
        d.set_item("class_major", site.class_major)?;
        d.set_item("up_class_major", site.up_class_major)?;
        d.set_item("parent_path", site.parent_path)?;
        sites_out.append(d)?;
    }
    out.set_item("sites", sites_out)?;

    let circuits_out = PyList::empty(py);
    for circuit in result.circuits {
        let d = PyDict::new(py);
        d.set_item("circuit_id", circuit.circuit_id)?;
        d.set_item("parent_node", circuit.parent_node)?;
        d.set_item("queue", circuit.queue)?;
        d.set_item("class_minor", circuit.class_minor)?;
        d.set_item("class_major", circuit.class_major)?;
        d.set_item("up_class_major", circuit.up_class_major)?;
        circuits_out.append(d)?;
    }
    out.set_item("circuits", circuits_out)?;

    let site_state_out = PyDict::new(py);
    for (key, state) in result.site_state {
        let d = PyDict::new(py);
        d.set_item("class_minor", state.class_minor)?;
        d.set_item("queue", state.queue)?;
        d.set_item("parent_path", state.parent_path)?;
        d.set_item("class_major", state.class_major)?;
        d.set_item("up_class_major", state.up_class_major)?;
        site_state_out.set_item(key, d)?;
    }
    out.set_item("site_state", site_state_out)?;

    let circuit_state_out = PyDict::new(py);
    for (key, state) in result.circuit_state {
        let d = PyDict::new(py);
        d.set_item("class_minor", state.class_minor)?;
        d.set_item("queue", state.queue)?;
        d.set_item("parent_node", state.parent_node)?;
        d.set_item("class_major", state.class_major)?;
        d.set_item("up_class_major", state.up_class_major)?;
        circuit_state_out.set_item(key, d)?;
    }
    out.set_item("circuit_state", circuit_state_out)?;

    let last_used = PyDict::new(py);
    for (queue, minor) in result.last_used_minor_by_queue {
        last_used.set_item(queue, minor)?;
    }
    out.set_item("last_used_minor_by_queue", last_used)?;
    Ok(out.into())
}

#[pyfunction]
fn write_planner_cbor(py: Python, path: String, state: PyObject) -> PyResult<bool> {
    use std::fs;
    use std::io::Write;
    let dict = state.cast_bound::<pyo3::types::PyDict>(py)?;
    // Build strongly typed struct, preserving integer keys
    let algo_version = get_string(dict, "algo_version", default_algo_version());
    let updated_at = get_f64(dict, "updated_at", 0.0);
    let queues_available = get_i64(dict, "queuesAvailable", 0);
    let on_a_stick = get_bool(dict, "on_a_stick", false);
    let site_count = get_i64(dict, "site_count", 0);
    let mut site_names: Vec<i64> = Vec::new();
    if let Ok(Some(sn)) = dict.get_item("site_names")
        && let Ok(list) = sn.cast::<pyo3::types::PyList>()
    {
        for item in list.iter() {
            if let Some(n) = to_i64_any(&item) {
                site_names.push(n);
            }
        }
    }
    // site_map
    let mut site_map: BTreeMap<i64, PlannerSiteEntry> = BTreeMap::new();
    if let Ok(Some(sm_any)) = dict.get_item("site_map")
        && let Ok(sm_dict) = sm_any.cast::<pyo3::types::PyDict>()
    {
        for (k, v) in sm_dict.iter() {
            if let Some(key) = to_i64_any(&k)
                && let Ok(entry) = v.cast::<pyo3::types::PyDict>()
            {
                let cpu = get_i64(entry, "cpu", 0);
                let major = get_i64(entry, "major", 0);
                let minor = get_i64(entry, "minor", 0);
                let insertion_order = match entry.get_item("insertion_order") {
                    Ok(Some(x)) => x.extract::<i64>().ok(),
                    _ => None,
                };
                site_map.insert(
                    key,
                    PlannerSiteEntry {
                        cpu,
                        major,
                        minor,
                        insertion_order,
                    },
                );
            }
        }
    }
    // circuit_map
    let mut circuit_map: BTreeMap<i64, PlannerCircuitEntry> = BTreeMap::new();
    if let Ok(Some(cm_any)) = dict.get_item("circuit_map")
        && let Ok(cm_dict) = cm_any.cast::<pyo3::types::PyDict>()
    {
        for (k, v) in cm_dict.iter() {
            if let Some(key) = to_i64_any(&k)
                && let Ok(entry) = v.cast::<pyo3::types::PyDict>()
            {
                let cpu = get_i64(entry, "cpu", 0);
                let major = get_i64(entry, "major", 0);
                let minor = get_i64(entry, "minor", 0);
                let parent_site = get_string(entry, "parent_site", String::new());
                let sqm = get_string(entry, "sqm", String::new());
                circuit_map.insert(
                    key,
                    PlannerCircuitEntry {
                        cpu,
                        major,
                        minor,
                        parent_site,
                        sqm,
                    },
                );
            }
        }
    }

    let to_save = PlannerStateSerde {
        algo_version,
        updated_at,
        queues_available,
        on_a_stick,
        site_count,
        site_names,
        site_map,
        circuit_map,
    };

    let cbor_bytes = serde_cbor::to_vec(&to_save).map_err(|e| {
        pyo3::exceptions::PyValueError::new_err(format!("CBOR encode failed: {e:?}"))
    })?;
    // Compress using the standard deflate scheme used elsewhere in the project
    let bytes = miniz_oxide::deflate::compress_to_vec(&cbor_bytes, 10);
    let path_tmp = format!("{}.tmp", &path);
    {
        let mut f = fs::File::create(&path_tmp).map_err(|e| {
            pyo3::exceptions::PyOSError::new_err(format!("Open {path_tmp} failed: {e:?}"))
        })?;
        f.write_all(&bytes).map_err(|e| {
            pyo3::exceptions::PyOSError::new_err(format!("Write {path_tmp} failed: {e:?}"))
        })?;
        f.flush().ok();
    }
    // Atomic replace
    fs::rename(&path_tmp, &path).map_err(|e| {
        pyo3::exceptions::PyOSError::new_err(format!("Rename {path_tmp} -> {path} failed: {e:?}"))
    })?;
    // Optionally, remove legacy JSON file – leave for migration
    Ok(true)
}

#[pyfunction]
fn read_planner_cbor(py: Python, path: String) -> PyResult<Option<PyObject>> {
    use std::fs;
    use std::path::Path;
    if !Path::new(&path).exists() {
        return Ok(None);
    }
    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(_) => return Ok(None),
    };
    // Attempt decompress-then-decode; fall back to raw CBOR for backward compatibility
    let decoded: PlannerStateSerde =
        if let Ok(decompressed) = miniz_oxide::inflate::decompress_to_vec(&bytes) {
            match serde_cbor::from_slice(&decompressed) {
                Ok(v) => v,
                Err(_) => return Ok(None),
            }
        } else {
            match serde_cbor::from_slice(&bytes) {
                Ok(v) => v,
                Err(_) => return Ok(None),
            }
        };
    // Convert to Python dict structure
    let out = pyo3::types::PyDict::new(py);
    out.set_item("algo_version", decoded.algo_version).ok();
    out.set_item("updated_at", decoded.updated_at).ok();
    out.set_item("queuesAvailable", decoded.queues_available)
        .ok();
    out.set_item("on_a_stick", decoded.on_a_stick).ok();
    out.set_item("site_count", decoded.site_count).ok();
    // site_names list
    let list = pyo3::types::PyList::new(py, decoded.site_names).unwrap();
    out.set_item("site_names", list).ok();
    // site_map
    let sm = pyo3::types::PyDict::new(py);
    for (k, v) in decoded.site_map.into_iter() {
        let entry = pyo3::types::PyDict::new(py);
        entry.set_item("cpu", v.cpu).ok();
        entry.set_item("major", v.major).ok();
        entry.set_item("minor", v.minor).ok();
        if let Some(ins) = v.insertion_order {
            entry.set_item("insertion_order", ins).ok();
        }
        sm.set_item(k, entry).ok();
    }
    out.set_item("site_map", sm).ok();
    // circuit_map
    let cm = pyo3::types::PyDict::new(py);
    for (k, v) in decoded.circuit_map.into_iter() {
        let entry = pyo3::types::PyDict::new(py);
        entry.set_item("cpu", v.cpu).ok();
        entry.set_item("major", v.major).ok();
        entry.set_item("minor", v.minor).ok();
        entry.set_item("parent_site", v.parent_site).ok();
        entry.set_item("sqm", v.sqm).ok();
        cm.set_item(k, entry).ok();
    }
    out.set_item("circuit_map", cm).ok();
    Ok(Some(out.into_any().unbind()))
}

// ===== Remote planner fetch/store over Insight web API =====

#[derive(Serialize, Deserialize)]
struct FetchPlannerRequest {
    org_key: String,
    node_id: String,
    queues_available: u32,
    on_a_stick: bool,
    site_count: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", content = "content")] // allow robust parsing if needed
enum FetchPlannerResponse {
    NoPlan,
    PlanBase64(String),
}

#[derive(Serialize, Deserialize)]
struct PersistPlannerRequest {
    org_key: String,
    node_id: String,
    queues_available: u32,
    on_a_stick: bool,
    site_count: u32,
    body_base64: String,
}

fn base_url_from_config() -> anyhow::Result<String> {
    let config = lqos_config::load_config()?;
    let base = config
        .long_term_stats
        .lts_url
        .clone()
        .unwrap_or_else(|| "insight.libreqos.com".to_string());
    Ok(format!("https://{}/shaper_api", base))
}

fn license_and_node_from_config() -> anyhow::Result<(String, String)> {
    let config = lqos_config::load_config()?;
    let org_key = config
        .long_term_stats
        .license_key
        .clone()
        .ok_or_else(|| anyhow::anyhow!("No Insight license key configured"))?;
    let node_id = config.node_id.clone();
    Ok((org_key, node_id))
}

#[pyfunction]
fn fetch_planner_remote(
    py: Python,
    queues_available: i64,
    on_a_stick: bool,
    site_count: i64,
) -> PyResult<Option<PyObject>> {
    // Build request
    let (org_key, node_id) = match license_and_node_from_config() {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    let base_url = match base_url_from_config() {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    let req = FetchPlannerRequest {
        org_key,
        node_id,
        queues_available: queues_available.max(0) as u32,
        on_a_stick,
        site_count: site_count.max(0) as u32,
    };
    let url = format!("{}/fetchPlanner", base_url);
    // Send
    // Allow invalid certs for self-hosted Insight instances (non-default URL)
    let allow_insecure = !base_url.starts_with("https://insight.libreqos.com");
    let client = match reqwest::blocking::Client::builder()
        // Use a generous timeout similar to deviceWeights to tolerate
        // slow or distant Insight instances when fetching planner state.
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(6))
        .danger_accept_invalid_certs(allow_insecure)
        .build()
    {
        Ok(c) => c,
        Err(_) => return Ok(None),
    };
    let resp = match client.post(&url).json(&req).send() {
        Ok(r) => r,
        Err(_) => return Ok(None),
    };
    if !resp.status().is_success() {
        return Ok(None);
    }
    let parsed: FetchPlannerResponse = match resp.json() {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    // Handle response
    match parsed {
        FetchPlannerResponse::NoPlan => Ok(None),
        FetchPlannerResponse::PlanBase64(b64) => {
            let Ok(bytes) = BASE64_STANDARD.decode(b64.as_bytes()) else {
                return Ok(None);
            };
            // Attempt decompress-then-decode; fall back to raw CBOR
            let decoded: PlannerStateSerde =
                if let Ok(decompressed) = miniz_oxide::inflate::decompress_to_vec(&bytes) {
                    match serde_cbor::from_slice(&decompressed) {
                        Ok(v) => v,
                        Err(_) => return Ok(None),
                    }
                } else {
                    match serde_cbor::from_slice(&bytes) {
                        Ok(v) => v,
                        Err(_) => return Ok(None),
                    }
                };
            // Convert to Python dict
            let out = pyo3::types::PyDict::new(py);
            out.set_item("algo_version", decoded.algo_version).ok();
            out.set_item("updated_at", decoded.updated_at).ok();
            out.set_item("queuesAvailable", decoded.queues_available)
                .ok();
            out.set_item("on_a_stick", decoded.on_a_stick).ok();
            out.set_item("site_count", decoded.site_count).ok();
            let list = pyo3::types::PyList::new(py, decoded.site_names).unwrap();
            out.set_item("site_names", list).ok();
            let sm = pyo3::types::PyDict::new(py);
            for (k, v) in decoded.site_map.into_iter() {
                let entry = pyo3::types::PyDict::new(py);
                entry.set_item("cpu", v.cpu).ok();
                entry.set_item("major", v.major).ok();
                entry.set_item("minor", v.minor).ok();
                if let Some(ins) = v.insertion_order {
                    entry.set_item("insertion_order", ins).ok();
                }
                sm.set_item(k, entry).ok();
            }
            out.set_item("site_map", sm).ok();
            let cm = pyo3::types::PyDict::new(py);
            for (k, v) in decoded.circuit_map.into_iter() {
                let entry = pyo3::types::PyDict::new(py);
                entry.set_item("cpu", v.cpu).ok();
                entry.set_item("major", v.major).ok();
                entry.set_item("minor", v.minor).ok();
                entry.set_item("parent_site", v.parent_site).ok();
                entry.set_item("sqm", v.sqm).ok();
                cm.set_item(k, entry).ok();
            }
            out.set_item("circuit_map", cm).ok();
            Ok(Some(out.into_any().unbind()))
        }
    }
}

#[pyfunction]
fn store_planner_remote(py: Python, state: PyObject) -> PyResult<bool> {
    // Extract needed values and serialize as compressed CBOR
    let dict = state.cast_bound::<pyo3::types::PyDict>(py)?;
    let algo_version = get_string(dict, "algo_version", default_algo_version());
    let updated_at = get_f64(dict, "updated_at", 0.0);
    let queues_available = get_i64(dict, "queuesAvailable", 0);
    let on_a_stick = get_bool(dict, "on_a_stick", false);
    let site_count = get_i64(dict, "site_count", 0);
    // site_names
    let mut site_names: Vec<i64> = Vec::new();
    if let Ok(Some(sn)) = dict.get_item("site_names")
        && let Ok(list) = sn.cast::<pyo3::types::PyList>()
    {
        for item in list.iter() {
            if let Some(n) = to_i64_any(&item) {
                site_names.push(n);
            }
        }
    }
    // site_map
    let mut site_map: BTreeMap<i64, PlannerSiteEntry> = BTreeMap::new();
    if let Ok(Some(sm_any)) = dict.get_item("site_map")
        && let Ok(sm_dict) = sm_any.cast::<pyo3::types::PyDict>()
    {
        for (k, v) in sm_dict.iter() {
            if let Some(key) = to_i64_any(&k)
                && let Ok(entry) = v.cast::<pyo3::types::PyDict>()
            {
                let cpu = get_i64(entry, "cpu", 0);
                let major = get_i64(entry, "major", 0);
                let minor = get_i64(entry, "minor", 0);
                let insertion_order = match entry.get_item("insertion_order") {
                    Ok(Some(x)) => x.extract::<i64>().ok(),
                    _ => None,
                };
                site_map.insert(
                    key,
                    PlannerSiteEntry {
                        cpu,
                        major,
                        minor,
                        insertion_order,
                    },
                );
            }
        }
    }
    // circuit_map
    let mut circuit_map: BTreeMap<i64, PlannerCircuitEntry> = BTreeMap::new();
    if let Ok(Some(cm_any)) = dict.get_item("circuit_map")
        && let Ok(cm_dict) = cm_any.cast::<pyo3::types::PyDict>()
    {
        for (k, v) in cm_dict.iter() {
            if let Some(key) = to_i64_any(&k)
                && let Ok(entry) = v.cast::<pyo3::types::PyDict>()
            {
                let cpu = get_i64(entry, "cpu", 0);
                let major = get_i64(entry, "major", 0);
                let minor = get_i64(entry, "minor", 0);
                let parent_site = get_string(entry, "parent_site", String::new());
                let sqm = get_string(entry, "sqm", String::new());
                circuit_map.insert(
                    key,
                    PlannerCircuitEntry {
                        cpu,
                        major,
                        minor,
                        parent_site,
                        sqm,
                    },
                );
            }
        }
    }
    let to_save = PlannerStateSerde {
        algo_version,
        updated_at,
        queues_available,
        on_a_stick,
        site_count,
        site_names,
        site_map,
        circuit_map,
    };
    let cbor_bytes = serde_cbor::to_vec(&to_save).map_err(|e| {
        pyo3::exceptions::PyValueError::new_err(format!("CBOR encode failed: {e:?}"))
    })?;
    let bytes = miniz_oxide::deflate::compress_to_vec(&cbor_bytes, 10);
    let body_base64 = BASE64_STANDARD.encode(&bytes);

    let (org_key, node_id) = match license_and_node_from_config() {
        Ok(v) => v,
        Err(_) => return Ok(false),
    };
    let base_url = match base_url_from_config() {
        Ok(v) => v,
        Err(_) => return Ok(false),
    };
    let url = format!("{}/storePlanner", base_url);
    let req = PersistPlannerRequest {
        org_key,
        node_id,
        queues_available: queues_available.max(0) as u32,
        on_a_stick,
        site_count: site_count.max(0) as u32,
        body_base64,
    };
    // Allow invalid certs for self-hosted Insight instances (non-default URL)
    let allow_insecure = !base_url.starts_with("https://insight.libreqos.com");
    let client = match reqwest::blocking::Client::builder()
        // Match the extended planner fetch timeout for symmetry.
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(6))
        .danger_accept_invalid_certs(allow_insecure)
        .build()
    {
        Ok(c) => c,
        Err(_) => return Ok(false),
    };
    match client.put(&url).json(&req).send() {
        Ok(resp) if resp.status().is_success() => Ok(true),
        _ => Ok(false),
    }
}

const LOCK_FILE: &str = "/run/lqos/libreqos.lock";

fn parse_topology_compile_mode(mode: &str) -> PyResult<TopologyCompileMode> {
    match mode.trim().to_lowercase().as_str() {
        "flat" => Ok(TopologyCompileMode::Flat),
        "ap_only" => Ok(TopologyCompileMode::ApOnly),
        "ap_site" => Ok(TopologyCompileMode::ApSite),
        "full" => Ok(TopologyCompileMode::Full),
        "full2" => Ok(TopologyCompileMode::Full2),
        _ => Err(PyOSError::new_err(format!(
            "Unknown topology compile mode '{mode}'"
        ))),
    }
}

fn write_compiled_topology_outputs(
    config: &lqos_config::Config,
    topology_import: &TopologyImportFile,
    compiled_shaping: &TopologyCompiledShapingFile,
    compiled: CompiledTopologyBundle,
) -> PyResult<()> {
    topology_import
        .save(config)
        .map_err(|e| PyOSError::new_err(format!("Unable to write topology_import.json: {e}")))?;
    compiled_shaping.save(config).map_err(|e| {
        PyOSError::new_err(format!(
            "Unable to write topology_compiled_shaping.json: {e}"
        ))
    })?;
    compiled.parent_candidates.save(config).map_err(|e| {
        PyOSError::new_err(format!(
            "Unable to write topology parent candidates snapshot: {e}"
        ))
    })?;
    // Rung 1.5 §1.3: topology_editor_state.json is never persisted.
    compiled.canonical.save(config).map_err(|e| {
        PyOSError::new_err(format!(
            "Unable to write topology canonical state snapshot: {e}"
        ))
    })?;
    let anchors_path = config.legacy_runtime_file_path(lqos_config::CIRCUIT_ANCHORS_FILENAME);
    if let Err(err) = remove_file(&anchors_path)
        && err.kind() != std::io::ErrorKind::NotFound
    {
        return Err(PyOSError::new_err(format!(
            "Unable to remove stale duplicate circuit_anchors.json at {}: {err}",
            anchors_path.display()
        )));
    }
    Ok(())
}

/// Atomically persists the topology canonical state from a JSON payload
/// (Phase C S1b).
///
/// The scheduler used to write `topology_canonical_state.json` with a bare
/// Python `open()`, bypassing the atomic write and the node validation. This
/// door keeps the runtime file owned by the Rust saver: parse → validate →
/// atomic rename.
#[pyfunction]
fn persist_topology_canonical_state(canonical_state_json: String) -> PyResult<()> {
    let Ok(config) = lqos_config::load_config() else {
        return Err(PyOSError::new_err("Unable to load configuration"));
    };
    let state: lqos_config::TopologyCanonicalStateFile =
        serde_json::from_str(&canonical_state_json).map_err(|e| {
            PyOSError::new_err(format!("Unable to parse topology canonical state: {e}"))
        })?;
    state.save(config.as_ref()).map_err(|e| {
        PyOSError::new_err(format!("Unable to write topology canonical state: {e}"))
    })
}

/// Defines the Python module exports.
/// All exported functions have to be listed here.
#[pymodule]
fn liblqos_python(m: &Bound<'_, PyModule>) -> PyResult<()> {    ensure_rustls_crypto_provider().map_err(|e| PyOSError::new_err(e.to_string()))?;
    m.add_class::<PyExceptionCpe>()?;
    m.add_class::<device_weights::DeviceWeightResponse>()?;
    m.add_function(wrap_pyfunction!(is_lqosd_alive, m)?)?;
    m.add_function(wrap_pyfunction!(sync_lqosd_config_from_disk, m)?)?;
    m.add_function(wrap_pyfunction!(validate_shaped_devices, m)?)?;
    m.add_function(wrap_pyfunction!(wait_for_bus_ready, m)?)?;
    m.add_function(wrap_pyfunction!(is_libre_already_running, m)?)?;
    m.add_function(wrap_pyfunction!(create_lock_file, m)?)?;
    m.add_function(wrap_pyfunction!(free_lock_file, m)?)?;
    // Unified configuration items
    m.add_function(wrap_pyfunction!(check_config, m)?)?;
    m.add_function(wrap_pyfunction!(sqm, m)?)?;
    m.add_function(wrap_pyfunction!(
        upstream_bandwidth_capacity_download_mbps,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        upstream_bandwidth_capacity_upload_mbps,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(interface_a, m)?)?;
    m.add_function(wrap_pyfunction!(interface_b, m)?)?;
    m.add_function(wrap_pyfunction!(enable_actual_shell_commands, m)?)?;
    m.add_function(wrap_pyfunction!(use_bin_packing_to_balance_cpu, m)?)?;
    m.add_function(wrap_pyfunction!(queue_mode, m)?)?;
    m.add_function(wrap_pyfunction!(shaping_cpu_count, m)?)?;
    m.add_function(wrap_pyfunction!(efficiency_core_ids, m)?)?;
    m.add_function(wrap_pyfunction!(run_shell_commands_as_sudo, m)?)?;
    m.add_function(wrap_pyfunction!(generated_pn_download_mbps, m)?)?;
    m.add_function(wrap_pyfunction!(generated_pn_upload_mbps, m)?)?;
    m.add_function(wrap_pyfunction!(queues_available_override, m)?)?;
    m.add_function(wrap_pyfunction!(on_a_stick, m)?)?;
    m.add_function(wrap_pyfunction!(migrate_legacy_site_bandwidth_csv, m)?)?;
    m.add_function(wrap_pyfunction!(allowed_subnets, m)?)?;
    m.add_function(wrap_pyfunction!(ignore_subnets, m)?)?;
    m.add_function(wrap_pyfunction!(circuit_name_use_address, m)?)?;
    m.add_function(wrap_pyfunction!(find_ipv6_using_mikrotik, m)?)?;
    m.add_function(wrap_pyfunction!(integration_common_use_mikrotik_ipv6, m)?)?;
    m.add_function(wrap_pyfunction!(mikrotik_ipv6_config_path, m)?)?;
    m.add_function(wrap_pyfunction!(load_mikrotik_ipv6_routers_json, m)?)?;
    m.add_function(wrap_pyfunction!(exclude_sites, m)?)?;
    m.add_function(wrap_pyfunction!(bandwidth_overhead_factor, m)?)?;
    m.add_function(wrap_pyfunction!(committed_bandwidth_multiplier, m)?)?;
    m.add_function(wrap_pyfunction!(exception_cpes, m)?)?;
    m.add_function(wrap_pyfunction!(uisp_site, m)?)?;
    m.add_function(wrap_pyfunction!(uisp_strategy, m)?)?;
    m.add_function(wrap_pyfunction!(uisp_suspended_strategy, m)?)?;
    m.add_function(wrap_pyfunction!(airmax_capacity, m)?)?;
    m.add_function(wrap_pyfunction!(ltu_capacity, m)?)?;
    m.add_function(wrap_pyfunction!(uisp_base_url, m)?)?;
    m.add_function(wrap_pyfunction!(uisp_auth_token, m)?)?;
    m.add_function(wrap_pyfunction!(splynx_api_key, m)?)?;
    m.add_function(wrap_pyfunction!(splynx_api_secret, m)?)?;
    m.add_function(wrap_pyfunction!(splynx_api_url, m)?)?;
    m.add_function(wrap_pyfunction!(splynx_strategy, m)?)?;
    m.add_function(wrap_pyfunction!(sonar_strategy, m)?)?;
    m.add_function(wrap_pyfunction!(repair_runtime_state, m)?)?;
    m.add_function(wrap_pyfunction!(
        write_compiled_topology_from_legacy_artifacts,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        write_compiled_topology_from_network_json_payload,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        write_compiled_topology_from_python_graph_payload,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(persist_topology_canonical_state, m)?)?;
    m.add_function(wrap_pyfunction!(netzur_api_key, m)?)?;
    m.add_function(wrap_pyfunction!(netzur_api_url, m)?)?;
    m.add_function(wrap_pyfunction!(netzur_api_timeout, m)?)?;
    m.add_function(wrap_pyfunction!(visp_client_id, m)?)?;
    m.add_function(wrap_pyfunction!(visp_client_secret, m)?)?;
    m.add_function(wrap_pyfunction!(visp_username, m)?)?;
    m.add_function(wrap_pyfunction!(visp_password, m)?)?;
    m.add_function(wrap_pyfunction!(visp_isp_id, m)?)?;
    m.add_function(wrap_pyfunction!(visp_online_users_domain, m)?)?;
    m.add_function(wrap_pyfunction!(visp_timeout_secs, m)?)?;
    m.add_function(wrap_pyfunction!(automatic_import_uisp, m)?)?;
    m.add_function(wrap_pyfunction!(automatic_import_splynx, m)?)?;
    m.add_function(wrap_pyfunction!(automatic_import_netzur, m)?)?;
    m.add_function(wrap_pyfunction!(automatic_import_visp, m)?)?;
    m.add_function(wrap_pyfunction!(queue_refresh_interval_mins, m)?)?;
    m.add_function(wrap_pyfunction!(automatic_import_powercode, m)?)?;
    m.add_function(wrap_pyfunction!(powercode_api_key, m)?)?;
    m.add_function(wrap_pyfunction!(powercode_api_url, m)?)?;
    m.add_function(wrap_pyfunction!(automatic_import_sonar, m)?)?;
    m.add_function(wrap_pyfunction!(sonar_api_url, m)?)?;
    m.add_function(wrap_pyfunction!(sonar_api_key, m)?)?;
    m.add_function(wrap_pyfunction!(snmp_community, m)?)?;
    m.add_function(wrap_pyfunction!(sonar_airmax_ap_model_ids, m)?)?;
    m.add_function(wrap_pyfunction!(sonar_ltu_ap_model_ids, m)?)?;
    m.add_function(wrap_pyfunction!(sonar_active_status_ids, m)?)?;
    m.add_function(wrap_pyfunction!(sonar_recurring_service_rates, m)?)?;
    m.add_function(wrap_pyfunction!(sonar_recurring_excluded_service_names, m)?)?;
    m.add_function(wrap_pyfunction!(influx_db_enabled, m)?)?;
    m.add_function(wrap_pyfunction!(influx_db_bucket, m)?)?;
    m.add_function(wrap_pyfunction!(influx_db_org, m)?)?;
    m.add_function(wrap_pyfunction!(influx_db_token, m)?)?;
    m.add_function(wrap_pyfunction!(influx_db_url, m)?)?;
    m.add_function(wrap_pyfunction!(get_weights, m)?)?;
    m.add_function(wrap_pyfunction!(get_tree_weights, m)?)?;
    m.add_function(wrap_pyfunction!(get_libreqos_directory, m)?)?;
    m.add_function(wrap_pyfunction!(get_libreqos_state_directory, m)?)?;
    m.add_function(wrap_pyfunction!(overrides_persistent_devices, m)?)?;
    m.add_function(wrap_pyfunction!(overrides_persistent_devices_effective, m)?)?;
    m.add_function(wrap_pyfunction!(
        overrides_persistent_devices_materialized,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(overrides_circuit_adjustments, m)?)?;
    m.add_function(wrap_pyfunction!(
        overrides_circuit_adjustments_effective,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        overrides_circuit_adjustments_materialized,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(overrides_network_adjustments, m)?)?;
    m.add_function(wrap_pyfunction!(
        overrides_network_adjustments_effective,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        overrides_network_adjustments_materialized,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(overrides_materialized, m)?)?;
    m.add_function(wrap_pyfunction!(is_network_flat, m)?)?;
    m.add_function(wrap_pyfunction!(blackboard_finish, m)?)?;
    m.add_function(wrap_pyfunction!(blackboard_submit, m)?)?;
    m.add_function(wrap_pyfunction!(automatic_import_wispgate, m)?)?;
    m.add_function(wrap_pyfunction!(wispgate_api_token, m)?)?;
    m.add_function(wrap_pyfunction!(wispgate_api_url, m)?)?;
    m.add_function(wrap_pyfunction!(enable_insight_topology, m)?)?;
    m.add_function(wrap_pyfunction!(insight_topology_role, m)?)?;
    m.add_function(wrap_pyfunction!(promote_to_root_list, m)?)?;
    m.add_function(wrap_pyfunction!(client_bandwidth_multiplier, m)?)?;
    m.add_function(wrap_pyfunction!(validated_runtime_shaping_inputs_path, m)?)?;
    m.add_function(wrap_pyfunction!(verify_published_shaping_generation, m)?)?;
    m.add_function(wrap_pyfunction!(calculate_shaping_runtime_hash, m)?)?;
    m.add_function(wrap_pyfunction!(scheduler_progress, m)?)?;
    m.add_function(wrap_pyfunction!(scheduler_integration_status, m)?)?;
    m.add_function(wrap_pyfunction!(scheduler_alive, m)?)?;
    m.add_function(wrap_pyfunction!(scheduler_error, m)?)?;
    m.add_function(wrap_pyfunction!(scheduler_output, m)?)?;
    m.add_function(wrap_pyfunction!(submit_urgent_issue, m)?)?;
    m.add_function(wrap_pyfunction!(clear_urgent_issue_by_identity, m)?)?;
    m.add_function(wrap_pyfunction!(is_insight_enabled, m)?)?;
    m.add_function(wrap_pyfunction!(log_info, m)?)?;
    m.add_function(wrap_pyfunction!(hash_to_i64, m)?)?;
    // Planner remote fetch/store for Insight integration
    m.add_function(wrap_pyfunction!(fetch_planner_remote, m)?)?;
    m.add_function(wrap_pyfunction!(store_planner_remote, m)?)?;
    m.add_function(wrap_pyfunction!(write_planner_cbor, m)?)?;
    m.add_function(wrap_pyfunction!(read_planner_cbor, m)?)?;
    m.add_function(wrap_pyfunction!(plan_top_level_cpu_bins, m)?)?;
    m.add_function(wrap_pyfunction!(plan_class_identities, m)?)?;

    Ok(())
}

/// Check that `lqosd` is running.
///
/// Returns true if it is running, false otherwise.
#[pyfunction]
fn is_lqosd_alive(_py: Python) -> PyResult<bool> {
    if let Ok(reply) = run_query(vec![BusRequest::Ping]) {
        for resp in reply.iter() {
            if let BusResponse::Ack = resp {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Reloads `/etc/lqos.conf` from disk in the current process and pushes the same
/// config into the running `lqosd` process.
///
/// This is intended for local admin workflows and test harnesses that update the
/// config file out-of-band and then need `lqosd` to observe the new values before
/// issuing runtime actions.
#[pyfunction]
fn sync_lqosd_config_from_disk(_py: Python) -> PyResult<()> {
    lqos_config::clear_cached_config();
    let config = lqos_config::load_config()
        .map_err(|e| PyOSError::new_err(format!("Unable to load /etc/lqos.conf: {e}")))?;
    let responses = run_query(vec![BusRequest::UpdateLqosdConfig(Box::new(
        (*config).clone(),
    ))])
    .map_err(|e| PyOSError::new_err(format!("Unable to push config into lqosd: {e}")))?;
    if !responses
        .iter()
        .any(|response| matches!(response, BusResponse::Ack))
    {
        return Err(PyOSError::new_err(
            "lqosd did not acknowledge the config update request",
        ));
    }
    lqos_config::clear_cached_config();
    Ok(())
}

/// Requests Rust-side validation of `ShapedDevices.csv`
#[pyfunction]
fn validate_shaped_devices() -> PyResult<String> {
    let result = run_query_wait_for_bus(
        vec![BusRequest::ValidateShapedDevicesCsv],
        Duration::from_secs(5),
        Duration::from_millis(100),
    )
    .map_err(|e| PyOSError::new_err(format!("Unable to validate shaped devices: {e}")))?;
    for response in result.iter() {
        match response {
            BusResponse::Ack => return Ok("OK".to_string()),
            BusResponse::ShapedDevicesValidation(error) => return Ok(error.clone()),
            _ => {}
        }
    }
    Ok("".to_string())
}

/// Waits until the local `lqosd` bus is ready to answer requests.
///
/// This is intended for scheduler startup sequencing. It retries only for
/// short-lived socket or handshake errors while `lqosd` is still binding the
/// bus socket.
#[pyfunction(signature = (timeout_ms = 5000))]
fn wait_for_bus_ready(timeout_ms: u64) -> PyResult<bool> {
    let replies = run_query_wait_for_bus(
        vec![BusRequest::Ping],
        Duration::from_millis(timeout_ms),
        Duration::from_millis(100),
    )
    .map_err(|e| PyOSError::new_err(format!("Timed out waiting for lqosd bus readiness: {e}")))?;
    for response in replies {
        if let BusResponse::Ack = response {
            return Ok(true);
        }
    }
    Err(PyOSError::new_err(
        "Timed out waiting for lqosd bus readiness: bus did not acknowledge ping",
    ))
}

/// Returns a Python list of dictionaries representing persistent devices for ShapedDevices.csv
/// The dictionary keys mirror the normalized loader used in LibreQoS.py:
/// circuitID, circuitName, deviceID, deviceName, ParentNode, ParentNodeID, mac,
/// ipv4s (list[str]), ipv6s (list[str]), minDownload, minUpload, maxDownload,
/// maxUpload, comment, sqm.
#[pyfunction]
fn overrides_persistent_devices(py: Python<'_>) -> PyResult<Vec<PyObject>> {
    let overrides = match lqos_overrides::OverrideFile::load_read_only() {
        Ok(o) => o,
        Err(e) => return Err(PyOSError::new_err(e.to_string())),
    };

    let mut out: Vec<PyObject> = Vec::new();
    for dev in overrides.persistent_devices().iter() {
        let ipv4s: Vec<String> = dev
            .ipv4
            .iter()
            .map(|(ip, prefix)| format!("{}/{}", ip, prefix))
            .collect();
        let ipv6s: Vec<String> = dev
            .ipv6
            .iter()
            .map(|(ip, prefix)| format!("{}/{}", ip, prefix))
            .collect();

        let d = PyDict::new(py);
        d.set_item("circuitID", dev.circuit_id.clone())?;
        d.set_item("circuitName", dev.circuit_name.clone())?;
        d.set_item("deviceID", dev.device_id.clone())?;
        d.set_item("deviceName", dev.device_name.clone())?;
        d.set_item("ParentNode", dev.parent_node.clone())?;
        d.set_item(
            "ParentNodeID",
            dev.parent_node_id.clone().unwrap_or_default(),
        )?;
        d.set_item(
            "AnchorNodeID",
            dev.anchor_node_id.clone().unwrap_or_default(),
        )?;
        d.set_item("mac", dev.mac.clone())?;
        d.set_item("ipv4s", ipv4s)?;
        d.set_item("ipv6s", ipv6s)?;
        d.set_item("minDownload", dev.download_min_mbps)?;
        d.set_item("minUpload", dev.upload_min_mbps)?;
        d.set_item("maxDownload", dev.download_max_mbps)?;
        d.set_item("maxUpload", dev.upload_max_mbps)?;
        d.set_item("comment", dev.comment.clone())?;
        d.set_item(
            "sqm",
            dev.sqm_override
                .as_ref()
                .map(|s| s.to_string())
                .unwrap_or_default(),
        )?;
        let obj: PyObject = d.unbind().into();
        out.push(obj);
    }

    Ok(out)
}

/// Returns a Python list of dictionaries representing persistent devices for ShapedDevices.csv,
/// using the effective overrides view (operator + adaptive layers when enabled).
#[pyfunction]
fn overrides_persistent_devices_effective(py: Python<'_>) -> PyResult<Vec<PyObject>> {
    let config = lqos_config::load_config().map_err(|e| PyOSError::new_err(e.to_string()))?;
    let apply_stormguard = config
        .stormguard
        .as_ref()
        .is_some_and(|sg| sg.enabled && !sg.dry_run);

    let overrides =
        match lqos_overrides::OverrideStore::load_effective_read_only(apply_stormguard) {
            Ok(o) => o,
            Err(e) => return Err(PyOSError::new_err(e.to_string())),
        };

    let mut out: Vec<PyObject> = Vec::new();
    for dev in overrides.persistent_devices().iter() {
        let ipv4s: Vec<String> = dev
            .ipv4
            .iter()
            .map(|(ip, prefix)| format!("{}/{}", ip, prefix))
            .collect();
        let ipv6s: Vec<String> = dev
            .ipv6
            .iter()
            .map(|(ip, prefix)| format!("{}/{}", ip, prefix))
            .collect();

        let d = PyDict::new(py);
        d.set_item("circuitID", dev.circuit_id.clone())?;
        d.set_item("circuitName", dev.circuit_name.clone())?;
        d.set_item("deviceID", dev.device_id.clone())?;
        d.set_item("deviceName", dev.device_name.clone())?;
        d.set_item("ParentNode", dev.parent_node.clone())?;
        d.set_item(
            "ParentNodeID",
            dev.parent_node_id.clone().unwrap_or_default(),
        )?;
        d.set_item(
            "AnchorNodeID",
            dev.anchor_node_id.clone().unwrap_or_default(),
        )?;
        d.set_item("mac", dev.mac.clone())?;
        d.set_item("ipv4s", ipv4s)?;
        d.set_item("ipv6s", ipv6s)?;
        d.set_item("minDownload", dev.download_min_mbps)?;
        d.set_item("minUpload", dev.upload_min_mbps)?;
        d.set_item("maxDownload", dev.download_max_mbps)?;
        d.set_item("maxUpload", dev.upload_max_mbps)?;
        d.set_item("comment", dev.comment.clone())?;
        d.set_item(
            "sqm",
            dev.sqm_override
                .as_ref()
                .map(|s| s.to_string())
                .unwrap_or_default(),
        )?;
        let obj: PyObject = d.unbind().into();
        out.push(obj);
    }

    Ok(out)
}

/// Returns a Python list of dictionaries representing persistent devices that should be
/// materialized into `ShapedDevices.csv`.
///
/// This includes only operator-owned persistent devices so adaptive runtime layers do not
/// overwrite the source-of-truth CSV.
#[pyfunction]
fn overrides_persistent_devices_materialized(py: Python<'_>) -> PyResult<Vec<PyObject>> {
    let overrides = load_materialized_overrides()?;

    persistent_devices_to_py(py, overrides.persistent_devices())
}

fn persistent_devices_to_py(
    py: Python<'_>,
    devices: &[lqos_config::ShapedDevice],
) -> PyResult<Vec<PyObject>> {
    let mut out: Vec<PyObject> = Vec::new();
    for dev in devices.iter() {
        let ipv4s: Vec<String> = dev
            .ipv4
            .iter()
            .map(|(ip, prefix)| format!("{}/{}", ip, prefix))
            .collect();
        let ipv6s: Vec<String> = dev
            .ipv6
            .iter()
            .map(|(ip, prefix)| format!("{}/{}", ip, prefix))
            .collect();

        let d = PyDict::new(py);
        d.set_item("circuitID", dev.circuit_id.clone())?;
        d.set_item("circuitName", dev.circuit_name.clone())?;
        d.set_item("deviceID", dev.device_id.clone())?;
        d.set_item("deviceName", dev.device_name.clone())?;
        d.set_item("ParentNode", dev.parent_node.clone())?;
        d.set_item(
            "ParentNodeID",
            dev.parent_node_id.clone().unwrap_or_default(),
        )?;
        d.set_item(
            "AnchorNodeID",
            dev.anchor_node_id.clone().unwrap_or_default(),
        )?;
        d.set_item("mac", dev.mac.clone())?;
        d.set_item("ipv4s", ipv4s)?;
        d.set_item("ipv6s", ipv6s)?;
        d.set_item("minDownload", dev.download_min_mbps)?;
        d.set_item("minUpload", dev.upload_min_mbps)?;
        d.set_item("maxDownload", dev.download_max_mbps)?;
        d.set_item("maxUpload", dev.upload_max_mbps)?;
        d.set_item("comment", dev.comment.clone())?;
        d.set_item(
            "sqm",
            dev.sqm_override
                .as_ref()
                .map(|s| s.to_string())
                .unwrap_or_default(),
        )?;
        let obj: PyObject = d.unbind().into();
        out.push(obj);
    }

    Ok(out)
}

/// Returns the list of circuit adjustments as Python dicts.
#[pyfunction]
fn overrides_circuit_adjustments(py: Python<'_>) -> PyResult<Vec<PyObject>> {
    let overrides = match lqos_overrides::OverrideFile::load_read_only() {
        Ok(o) => o,
        Err(e) => return Err(PyOSError::new_err(e.to_string())),
    };

    let mut out: Vec<PyObject> = Vec::new();
    for adj in overrides.circuit_adjustments().iter() {
        let d = PyDict::new(py);
        match adj {
            lqos_overrides::CircuitAdjustment::CircuitAdjustSpeed {
                circuit_id,
                min_download_bandwidth,
                max_download_bandwidth,
                min_upload_bandwidth,
                max_upload_bandwidth,
            } => {
                d.set_item("type", "circuit_adjust_speed")?;
                d.set_item("circuit_id", circuit_id.clone())?;
                if let Some(v) = min_download_bandwidth {
                    d.set_item("min_download_bandwidth", *v)?;
                }
                if let Some(v) = max_download_bandwidth {
                    d.set_item("max_download_bandwidth", *v)?;
                }
                if let Some(v) = min_upload_bandwidth {
                    d.set_item("min_upload_bandwidth", *v)?;
                }
                if let Some(v) = max_upload_bandwidth {
                    d.set_item("max_upload_bandwidth", *v)?;
                }
            }
            lqos_overrides::CircuitAdjustment::DeviceAdjustSpeed {
                device_id,
                min_download_bandwidth,
                max_download_bandwidth,
                min_upload_bandwidth,
                max_upload_bandwidth,
            } => {
                d.set_item("type", "device_adjust_speed")?;
                d.set_item("device_id", device_id.clone())?;
                if let Some(v) = min_download_bandwidth {
                    d.set_item("min_download_bandwidth", *v)?;
                }
                if let Some(v) = max_download_bandwidth {
                    d.set_item("max_download_bandwidth", *v)?;
                }
                if let Some(v) = min_upload_bandwidth {
                    d.set_item("min_upload_bandwidth", *v)?;
                }
                if let Some(v) = max_upload_bandwidth {
                    d.set_item("max_upload_bandwidth", *v)?;
                }
            }
            lqos_overrides::CircuitAdjustment::DeviceAdjustSqm {
                device_id,
                sqm_override,
            } => {
                d.set_item("type", "device_adjust_sqm")?;
                d.set_item("device_id", device_id.clone())?;
                if let Some(value) = sqm_override {
                    d.set_item("sqm_override", value.clone())?;
                }
            }
            lqos_overrides::CircuitAdjustment::CircuitAdjustAqm {
                circuit_id,
                aqm_token,
                expires_at_unix,
            } => {
                d.set_item("type", "circuit_adjust_aqm")?;
                d.set_item("circuit_id", circuit_id.clone())?;
                d.set_item("aqm_token", aqm_token.clone())?;
                if let Some(value) = expires_at_unix {
                    d.set_item("expires_at_unix", *value)?;
                }
            }
            lqos_overrides::CircuitAdjustment::RemoveCircuit { circuit_id } => {
                d.set_item("type", "remove_circuit")?;
                d.set_item("circuit_id", circuit_id.clone())?;
            }
            lqos_overrides::CircuitAdjustment::RemoveDevice { device_id } => {
                d.set_item("type", "remove_device")?;
                d.set_item("device_id", device_id.clone())?;
            }
            lqos_overrides::CircuitAdjustment::ReparentCircuit {
                circuit_id,
                parent_node,
            } => {
                d.set_item("type", "reparent_circuit")?;
                d.set_item("circuit_id", circuit_id.clone())?;
                d.set_item("parent_node", parent_node.clone())?;
            }
        }
        let obj: PyObject = d.unbind().into();
        out.push(obj);
    }

    Ok(out)
}

/// Returns the list of circuit adjustments as Python dicts, using the effective overrides view
/// (operator + adaptive layers when enabled).
#[pyfunction]
fn overrides_circuit_adjustments_effective(py: Python<'_>) -> PyResult<Vec<PyObject>> {
    let config = lqos_config::load_config().map_err(|e| PyOSError::new_err(e.to_string()))?;
    let apply_stormguard = config
        .stormguard
        .as_ref()
        .is_some_and(|sg| sg.enabled && !sg.dry_run);

    let overrides =
        match lqos_overrides::OverrideStore::load_effective_read_only(apply_stormguard) {
            Ok(o) => o,
            Err(e) => return Err(PyOSError::new_err(e.to_string())),
        };

    let mut out: Vec<PyObject> = Vec::new();
    for adj in overrides.circuit_adjustments().iter() {
        let d = PyDict::new(py);
        match adj {
            lqos_overrides::CircuitAdjustment::CircuitAdjustSpeed {
                circuit_id,
                min_download_bandwidth,
                max_download_bandwidth,
                min_upload_bandwidth,
                max_upload_bandwidth,
            } => {
                d.set_item("type", "circuit_adjust_speed")?;
                d.set_item("circuit_id", circuit_id.clone())?;
                if let Some(v) = min_download_bandwidth {
                    d.set_item("min_download_bandwidth", *v)?;
                }
                if let Some(v) = max_download_bandwidth {
                    d.set_item("max_download_bandwidth", *v)?;
                }
                if let Some(v) = min_upload_bandwidth {
                    d.set_item("min_upload_bandwidth", *v)?;
                }
                if let Some(v) = max_upload_bandwidth {
                    d.set_item("max_upload_bandwidth", *v)?;
                }
            }
            lqos_overrides::CircuitAdjustment::DeviceAdjustSpeed {
                device_id,
                min_download_bandwidth,
                max_download_bandwidth,
                min_upload_bandwidth,
                max_upload_bandwidth,
            } => {
                d.set_item("type", "device_adjust_speed")?;
                d.set_item("device_id", device_id.clone())?;
                if let Some(v) = min_download_bandwidth {
                    d.set_item("min_download_bandwidth", *v)?;
                }
                if let Some(v) = max_download_bandwidth {
                    d.set_item("max_download_bandwidth", *v)?;
                }
                if let Some(v) = min_upload_bandwidth {
                    d.set_item("min_upload_bandwidth", *v)?;
                }
                if let Some(v) = max_upload_bandwidth {
                    d.set_item("max_upload_bandwidth", *v)?;
                }
            }
            lqos_overrides::CircuitAdjustment::DeviceAdjustSqm {
                device_id,
                sqm_override,
            } => {
                d.set_item("type", "device_adjust_sqm")?;
                d.set_item("device_id", device_id.clone())?;
                if let Some(value) = sqm_override {
                    d.set_item("sqm_override", value.clone())?;
                }
            }
            lqos_overrides::CircuitAdjustment::CircuitAdjustAqm {
                circuit_id,
                aqm_token,
                expires_at_unix,
            } => {
                d.set_item("type", "circuit_adjust_aqm")?;
                d.set_item("circuit_id", circuit_id.clone())?;
                d.set_item("aqm_token", aqm_token.clone())?;
                if let Some(value) = expires_at_unix {
                    d.set_item("expires_at_unix", *value)?;
                }
            }
            lqos_overrides::CircuitAdjustment::RemoveCircuit { circuit_id } => {
                d.set_item("type", "remove_circuit")?;
                d.set_item("circuit_id", circuit_id.clone())?;
            }
            lqos_overrides::CircuitAdjustment::RemoveDevice { device_id } => {
                d.set_item("type", "remove_device")?;
                d.set_item("device_id", device_id.clone())?;
            }
            lqos_overrides::CircuitAdjustment::ReparentCircuit {
                circuit_id,
                parent_node,
            } => {
                d.set_item("type", "reparent_circuit")?;
                d.set_item("circuit_id", circuit_id.clone())?;
                d.set_item("parent_node", parent_node.clone())?;
            }
        }
        let obj: PyObject = d.unbind().into();
        out.push(obj);
    }

    Ok(out)
}

/// Returns the list of circuit adjustments that should be materialized into `ShapedDevices.csv`.
///
/// This includes only operator-owned circuit adjustments so adaptive runtime layers do not
/// overwrite the source-of-truth CSV.
#[pyfunction]
fn overrides_circuit_adjustments_materialized(py: Python<'_>) -> PyResult<Vec<PyObject>> {
    let overrides = load_materialized_overrides()?;

    circuit_adjustments_to_py(py, overrides.circuit_adjustments())
}

fn circuit_adjustments_to_py(
    py: Python<'_>,
    adjustments: &[lqos_overrides::CircuitAdjustment],
) -> PyResult<Vec<PyObject>> {
    let mut out: Vec<PyObject> = Vec::new();
    for adj in adjustments.iter() {
        let d = PyDict::new(py);
        match adj {
            lqos_overrides::CircuitAdjustment::CircuitAdjustSpeed {
                circuit_id,
                min_download_bandwidth,
                max_download_bandwidth,
                min_upload_bandwidth,
                max_upload_bandwidth,
            } => {
                d.set_item("type", "circuit_adjust_speed")?;
                d.set_item("circuit_id", circuit_id.clone())?;
                if let Some(v) = min_download_bandwidth {
                    d.set_item("min_download_bandwidth", *v)?;
                }
                if let Some(v) = max_download_bandwidth {
                    d.set_item("max_download_bandwidth", *v)?;
                }
                if let Some(v) = min_upload_bandwidth {
                    d.set_item("min_upload_bandwidth", *v)?;
                }
                if let Some(v) = max_upload_bandwidth {
                    d.set_item("max_upload_bandwidth", *v)?;
                }
            }
            lqos_overrides::CircuitAdjustment::DeviceAdjustSpeed {
                device_id,
                min_download_bandwidth,
                max_download_bandwidth,
                min_upload_bandwidth,
                max_upload_bandwidth,
            } => {
                d.set_item("type", "device_adjust_speed")?;
                d.set_item("device_id", device_id.clone())?;
                if let Some(v) = min_download_bandwidth {
                    d.set_item("min_download_bandwidth", *v)?;
                }
                if let Some(v) = max_download_bandwidth {
                    d.set_item("max_download_bandwidth", *v)?;
                }
                if let Some(v) = min_upload_bandwidth {
                    d.set_item("min_upload_bandwidth", *v)?;
                }
                if let Some(v) = max_upload_bandwidth {
                    d.set_item("max_upload_bandwidth", *v)?;
                }
            }
            lqos_overrides::CircuitAdjustment::DeviceAdjustSqm {
                device_id,
                sqm_override,
            } => {
                d.set_item("type", "device_adjust_sqm")?;
                d.set_item("device_id", device_id.clone())?;
                if let Some(value) = sqm_override {
                    d.set_item("sqm_override", value.clone())?;
                }
            }
            lqos_overrides::CircuitAdjustment::CircuitAdjustAqm {
                circuit_id,
                aqm_token,
                expires_at_unix,
            } => {
                d.set_item("type", "circuit_adjust_aqm")?;
                d.set_item("circuit_id", circuit_id.clone())?;
                d.set_item("aqm_token", aqm_token.clone())?;
                if let Some(value) = expires_at_unix {
                    d.set_item("expires_at_unix", *value)?;
                }
            }
            lqos_overrides::CircuitAdjustment::RemoveCircuit { circuit_id } => {
                d.set_item("type", "remove_circuit")?;
                d.set_item("circuit_id", circuit_id.clone())?;
            }
            lqos_overrides::CircuitAdjustment::RemoveDevice { device_id } => {
                d.set_item("type", "remove_device")?;
                d.set_item("device_id", device_id.clone())?;
            }
            lqos_overrides::CircuitAdjustment::ReparentCircuit {
                circuit_id,
                parent_node,
            } => {
                d.set_item("type", "reparent_circuit")?;
                d.set_item("circuit_id", circuit_id.clone())?;
                d.set_item("parent_node", parent_node.clone())?;
            }
        }
        let obj: PyObject = d.unbind().into();
        out.push(obj);
    }

    Ok(out)
}

/// Returns the list of network adjustments as Python dicts.
#[pyfunction]
fn overrides_network_adjustments(py: Python<'_>) -> PyResult<Vec<PyObject>> {
    let overrides = match lqos_overrides::OverrideFile::load_read_only() {
        Ok(o) => o,
        Err(e) => return Err(PyOSError::new_err(e.to_string())),
    };

    network_adjustments_to_py(py, overrides.network_adjustments())
}

/// Returns the list of network adjustments as Python dicts, using the effective overrides view
/// (operator + adaptive layers when enabled).
#[pyfunction]
fn overrides_network_adjustments_effective(py: Python<'_>) -> PyResult<Vec<PyObject>> {
    let config = lqos_config::load_config().map_err(|e| PyOSError::new_err(e.to_string()))?;
    let apply_stormguard = config
        .stormguard
        .as_ref()
        .is_some_and(|sg| sg.enabled && !sg.dry_run);

    let overrides =
        match lqos_overrides::OverrideStore::load_effective_read_only(apply_stormguard) {
            Ok(o) => o,
            Err(e) => return Err(PyOSError::new_err(e.to_string())),
        };

    network_adjustments_to_py(py, overrides.network_adjustments())
}

/// Returns the list of network adjustments that should be materialized into `network.json`.
///
/// This includes only operator-owned network adjustments.
///
/// StormGuard adaptive site-speed decisions are intentionally excluded so runtime
/// automation does not overwrite the operator-authored topology/source-of-truth
/// file.
#[pyfunction]
fn overrides_network_adjustments_materialized(py: Python<'_>) -> PyResult<Vec<PyObject>> {
    let overrides = load_materialized_overrides()?;

    network_adjustments_to_py(py, overrides.network_adjustments())
}

/// Returns all operator-owned override sections that scheduler materializes into
/// source-of-truth compatibility files.
///
/// Side effects: acquires the overrides file lock once and reads the effective
/// operator-only override snapshot.
#[pyfunction]
fn overrides_materialized(py: Python<'_>) -> PyResult<PyObject> {
    let overrides = load_materialized_overrides()?;

    let out = PyDict::new(py);
    out.set_item(
        "persistent_devices",
        persistent_devices_to_py(py, overrides.persistent_devices())?,
    )?;
    out.set_item(
        "circuit_adjustments",
        circuit_adjustments_to_py(py, overrides.circuit_adjustments())?,
    )?;
    out.set_item(
        "network_adjustments",
        network_adjustments_to_py(py, overrides.network_adjustments())?,
    )?;
    Ok(out.unbind().into())
}

fn load_materialized_overrides() -> PyResult<lqos_overrides::OverrideFile> {
    lqos_overrides::OverrideStore::load_effective_read_only(false)
        .map_err(|e| PyOSError::new_err(e.to_string()))
}

fn network_adjustments_to_py(
    py: Python<'_>,
    adjustments: &[lqos_overrides::NetworkAdjustment],
) -> PyResult<Vec<PyObject>> {
    let mut out: Vec<PyObject> = Vec::new();
    for adj in adjustments.iter() {
        let d = PyDict::new(py);
        match adj {
            lqos_overrides::NetworkAdjustment::AdjustSiteSpeed {
                node_id,
                site_name,
                download_bandwidth_mbps,
                upload_bandwidth_mbps,
                expires_at_unix,
            } => {
                d.set_item("type", "adjust_site_speed")?;
                if let Some(node_id) = node_id {
                    d.set_item("node_id", node_id.clone())?;
                }
                d.set_item("site_name", site_name.clone())?;
                if let Some(v) = download_bandwidth_mbps {
                    d.set_item("download_bandwidth_mbps", *v)?;
                }
                if let Some(v) = upload_bandwidth_mbps {
                    d.set_item("upload_bandwidth_mbps", *v)?;
                }
                if let Some(expires_at_unix) = expires_at_unix {
                    d.set_item("expires_at_unix", *expires_at_unix)?;
                }
            }
            lqos_overrides::NetworkAdjustment::TopologyParentOverride {
                node_id,
                node_name,
                mode,
                parent_node_ids,
                parent_node_names,
            } => {
                d.set_item("type", "topology_parent_override")?;
                d.set_item("node_id", node_id.clone())?;
                d.set_item("node_name", node_name.clone())?;
                d.set_item(
                    "mode",
                    match mode {
                        lqos_overrides::TopologyParentOverrideMode::Pinned => "pinned",
                        lqos_overrides::TopologyParentOverrideMode::PreferredOrder => {
                            "preferred_order"
                        }
                    },
                )?;
                d.set_item("parent_node_ids", parent_node_ids.clone())?;
                d.set_item("parent_node_names", parent_node_names.clone())?;
            }
        }
        let obj: PyObject = d.unbind().into();
        out.push(obj);
    }

    Ok(out)
}

#[pyfunction]
fn is_libre_already_running() -> PyResult<bool> {
    let lock_path = Path::new(LOCK_FILE);
    if lock_path.exists() {
        let contents = std::fs::read_to_string(lock_path);
        if let Ok(contents) = contents {
            if let Ok(pid) = contents.parse::<i32>() {
                let sys = System::new_all();
                let pid = sysinfo::Pid::from(pid as usize);
                if let Some(process) = sys.processes().get(&pid)
                    && process.name().to_string_lossy().contains("python")
                {
                    return Ok(true);
                }
            } else {
                println!("{LOCK_FILE} did not contain a valid PID");
                return Ok(false);
            }
        } else {
            println!("Error reading contents of {LOCK_FILE}");
            return Ok(false);
        }
    }
    Ok(false)
}

#[pyfunction]
fn create_lock_file() -> PyResult<()> {
    let pid = unsafe { getpid() };
    let pid_format = format!("{pid}");
    {
        if let Ok(mut f) = File::create(LOCK_FILE) {
            f.write_all(pid_format.as_bytes())?;
        }
    }
    Ok(())
}

#[pyfunction]
fn free_lock_file() -> PyResult<()> {
    let _ = remove_file(LOCK_FILE); // Ignore result
    Ok(())
}

#[pyfunction]
fn check_config() -> PyResult<bool> {
    let config = lqos_config::load_config();
    if let Err(e) = config {
        println!("Error loading config: {e}");
        return Ok(false);
    }
    Ok(true)
}

#[pyfunction]
fn sqm() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.queues.default_sqm.clone())
}

/// The configured upstream capacity pair `(download, upload)` in Mbps.
/// `download` IS the down cap and `upload` IS the up cap — the mapping
/// `Config::bandwidth_mbps()` owns ([dataplane] preferred, [queues] fallback),
/// the SAME fact `throughput_tracker::stats_submission` passes to
/// `bridge_spike_guard(shaped.down, shaped.up, ...)`. Extracted so the mapping is
/// unit-testable without the global config (upstream #1151 returned the uplink
/// for the download accessor).
fn upstream_capacities_mbps(config: &lqos_config::Config) -> (u32, u32) {
    let (download, upload) = config.bandwidth_mbps();
    (download as u32, upload as u32)
}

#[pyfunction]
fn upstream_bandwidth_capacity_download_mbps() -> PyResult<u32> {
    let config = lqos_config::load_config().unwrap();
    Ok(upstream_capacities_mbps(&config).0)
}

#[pyfunction]
fn upstream_bandwidth_capacity_upload_mbps() -> PyResult<u32> {
    let config = lqos_config::load_config().unwrap();
    Ok(upstream_capacities_mbps(&config).1)
}

#[pyfunction]
fn interface_a() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.isp_interface())
}

#[pyfunction]
fn interface_b() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.internet_interface())
}

#[pyfunction]
fn enable_actual_shell_commands() -> PyResult<bool> {
    let config = lqos_config::load_config().unwrap();
    Ok(!config.queues.dry_run)
}

#[pyfunction]
fn use_bin_packing_to_balance_cpu() -> PyResult<bool> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.queues.use_binpacking)
}

#[pyfunction]
fn queue_mode() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    Ok(match config.queues.queue_mode {
        lqos_config::QueueMode::Shape => "shape".to_string(),
        lqos_config::QueueMode::Observe => "observe".to_string(),
    })
}

/// Returns the number of CPUs that should be used for shaping / binning.
///
/// On hybrid CPUs, this may exclude efficiency cores (E-cores) when configured
/// using the cached multi-method topology detector.
#[pyfunction]
fn shaping_cpu_count() -> PyResult<u32> {
    let config = lqos_config::load_config().unwrap();
    let det = lqos_config::detect_shaping_cpus(config.as_ref());
    Ok(det.shaping.len() as u32)
}

/// Returns detected efficiency-core CPU IDs for scheduler affinity decisions.
#[pyfunction]
fn efficiency_core_ids() -> PyResult<Vec<u32>> {
    let config = lqos_config::load_config().unwrap();
    let det = lqos_config::detect_shaping_cpus(config.as_ref());
    Ok(if det.has_hybrid_split {
        det.efficiency
    } else {
        Vec::new()
    })
}

#[pyfunction]
fn run_shell_commands_as_sudo() -> PyResult<bool> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.queues.sudo)
}

#[pyfunction]
fn generated_pn_download_mbps() -> PyResult<u32> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.queues.generated_pn_download_mbps as u32)
}

#[pyfunction]
fn generated_pn_upload_mbps() -> PyResult<u32> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.queues.generated_pn_upload_mbps as u32)
}

#[pyfunction]
fn queues_available_override() -> PyResult<u32> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.queues.override_available_queues.unwrap_or(0))
}

#[pyfunction]
fn on_a_stick() -> PyResult<bool> {
    // K3-B: single-interface (on-a-stick) mode is retired; the bridge is the
    // only dataplane. Retained as a constant `false` so the Python shaper
    // module imports cleanly while the kernel call sites are still present.
    Ok(false)
}

#[pyfunction]
fn allowed_subnets() -> PyResult<Vec<String>> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.ip_ranges.allow_subnets.clone())
}

#[pyfunction]
fn ignore_subnets() -> PyResult<Vec<String>> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.ip_ranges.ignore_subnets.clone())
}

#[pyfunction]
fn circuit_name_use_address() -> PyResult<bool> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.integration_common.circuit_name_as_address)
}

#[pyfunction]
fn find_ipv6_using_mikrotik() -> PyResult<bool> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.uisp_integration.ipv6_with_mikrotik || config.integration_common.use_mikrotik_ipv6)
}

#[pyfunction]
fn integration_common_use_mikrotik_ipv6() -> PyResult<bool> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.integration_common.use_mikrotik_ipv6)
}

#[pyfunction]
fn mikrotik_ipv6_config_path() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    Ok(config
        .resolved_mikrotik_ipv6_config_path()
        .display()
        .to_string())
}

#[pyfunction]
fn load_mikrotik_ipv6_routers_json() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    let routers = lqos_config::load_mikrotik_ipv6_router_credentials(&config)
        .map_err(|e| PyOSError::new_err(e.to_string()))?;
    serde_json::to_string(&routers).map_err(|e| PyOSError::new_err(e.to_string()))
}

#[pyfunction]
fn exclude_sites() -> PyResult<Vec<String>> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.uisp_integration.exclude_sites.clone())
}

#[pyfunction]
fn bandwidth_overhead_factor() -> PyResult<f32> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.uisp_integration.bandwidth_overhead_factor)
}

#[pyfunction]
fn committed_bandwidth_multiplier() -> PyResult<f32> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.uisp_integration.commit_bandwidth_multiplier)
}

#[pyclass]
/// A UISP exception CPE entry paired with its forced parent.
pub struct PyExceptionCpe {
    /// Child CPE site or device identifier.
    #[pyo3(get)]
    pub cpe: String,
    /// Parent identifier assigned to the exception CPE.
    #[pyo3(get)]
    pub parent: String,
}

#[pyfunction]
fn exception_cpes() -> PyResult<Vec<PyExceptionCpe>> {
    let config = lqos_config::load_config().unwrap();
    let mut result = Vec::new();
    for cpe in config.uisp_integration.exception_cpes.iter() {
        result.push(PyExceptionCpe {
            cpe: cpe.cpe.clone(),
            parent: cpe.parent.clone(),
        });
    }
    Ok(result)
}

#[pyfunction]
fn uisp_site() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    let site = config.uisp_integration.site.clone();
    Ok(site)
}

#[pyfunction]
fn uisp_strategy() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.resolved_topology_compile_mode_for_uisp().to_string())
}

#[pyfunction]
fn uisp_suspended_strategy() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    let strategy = config.uisp_integration.suspended_strategy.clone();
    Ok(strategy)
}

#[pyfunction]
fn airmax_capacity() -> PyResult<f32> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.uisp_integration.airmax_capacity)
}

#[pyfunction]
fn ltu_capacity() -> PyResult<f32> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.uisp_integration.ltu_capacity)
}

#[pyfunction]
fn uisp_base_url() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    let url = config.uisp_integration.url.clone();
    Ok(url)
}

#[pyfunction]
fn uisp_auth_token() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    let token = config.uisp_integration.token.clone();
    Ok(token)
}

#[pyfunction]
fn splynx_api_key() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    let key = config.splynx_integration.api_key.clone();
    Ok(key)
}

#[pyfunction]
fn splynx_api_secret() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    let secret = config.splynx_integration.api_secret.clone();
    Ok(secret)
}

#[pyfunction]
fn splynx_api_url() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    let url = config.splynx_integration.url.clone();
    Ok(url)
}

#[pyfunction]
fn splynx_strategy() -> PyResult<String> {
    let config = lqos_config::load_config();
    match config {
        Ok(config) => Ok(config
            .resolved_topology_compile_mode_for_splynx()
            .to_string()),
        Err(_) => Ok("ap_only".to_string()), // Default value when config can't be loaded
    }
}

#[pyfunction]
fn sonar_strategy() -> PyResult<String> {
    let config = lqos_config::load_config();
    match config {
        Ok(config) => Ok(config
            .resolved_topology_compile_mode_for_sonar()
            .to_string()),
        Err(_) => Ok("full".to_string()),
    }
}

/// Phase C S4: the scheduler's manual producer. Materializes the compiled
/// ingest artifacts AND the canonical projection from the permanent manual
/// pair through the SAME producer the runtime uses, so the two stay symmetric.
///
/// The `source`/`compile_mode` arguments are retained for the Python call
/// contract; the producer derives both from the config.
#[pyfunction]
fn write_compiled_topology_from_legacy_artifacts(
    source: String,
    compile_mode: String,
) -> PyResult<bool> {
    let _ = (source, compile_mode);
    let config = lqos_config::load_config().map_err(|e| PyOSError::new_err(e.to_string()))?;
    lqos_topology_compile::materialize_manual_topology(config.as_ref()).map_err(|e| {
        PyOSError::new_err(format!("Unable to materialize the manual topology: {e:#}"))
    })?;
    Ok(true)
}

/// Phase D: the scheduler's idempotent repair entry, run before materializing
/// the ingest artifacts. lqosd startup is the primary repair owner; this is the
/// maintenance call (the boot heals, the producer maintains).
#[pyfunction]
fn repair_runtime_state() -> PyResult<()> {
    let config = lqos_config::load_config().map_err(|e| PyOSError::new_err(e.to_string()))?;
    lqos_config::repair_runtime_state(config.as_ref())
        .map_err(|e| PyOSError::new_err(format!("runtime-state repair failed: {e}")))
}

/// Parses a standard `libreqos_network.json` payload into the imported bundle
/// the compiler consumes (Phase C S3c).
///
/// The compatibility `network.json` shape retires from this door: the format
/// reader is the new standard, and the legacy on-disk pair keeps
/// [`ImportedTopologyBundle::from_legacy_artifacts`]. Pure: no side effects.
fn imported_bundle_from_libreqos_network_payload(
    source: &str,
    libreqos_network_json: &str,
) -> Result<ImportedTopologyBundle, String> {
    let file = parse_libreqos_network(libreqos_network_json)
        .map_err(|error| format!("Unable to parse libreqos_network.json payload: {error}"))?;
    Ok(file.into_imported_bundle(source.to_string()))
}

#[pyfunction]
fn write_compiled_topology_from_network_json_payload(
    source: String,
    compile_mode: String,
    libreqos_network_json: String,
) -> PyResult<bool> {
    let config = lqos_config::load_config().map_err(|e| PyOSError::new_err(e.to_string()))?;
    let imported = imported_bundle_from_libreqos_network_payload(&source, &libreqos_network_json)
        .map_err(PyOSError::new_err)?;
    let topology_import = TopologyImportFile::from_imported_bundle(&imported, compile_mode.clone());
    let mode = parse_topology_compile_mode(&compile_mode)?;
    let compiled = compile_topology(imported, mode).map_err(|e| {
        PyOSError::new_err(format!(
            "Unable to compile topology payload for '{source}' using mode '{compile_mode}': {e}"
        ))
    })?;
    let compiled_shaping =
        TopologyCompiledShapingFile::from_compiled(&compiled, compile_mode.clone());
    write_compiled_topology_outputs(
        config.as_ref(),
        &topology_import,
        &compiled_shaping,
        compiled,
    )?;
    Ok(true)
}

#[pyfunction]
fn write_compiled_topology_from_python_graph_payload(
    source: String,
    compile_mode: String,
    compatibility_network_json: String,
    shaped_devices_csv: String,
    circuit_anchors_json: String,
    parent_candidates_json: String,
    native_editor_json: String,
) -> PyResult<bool> {
    let config = lqos_config::load_config().map_err(|e| PyOSError::new_err(e.to_string()))?;
    let compatibility_network_json =
        serde_json::from_str::<serde_json::Value>(&compatibility_network_json).map_err(|e| {
            PyOSError::new_err(format!("Unable to parse topology payload JSON: {e}"))
        })?;
    let shaped_devices =
        load_shaped_devices_from_csv_payload(&shaped_devices_csv).map_err(|e| {
            PyOSError::new_err(format!("Unable to parse shaped-device CSV payload: {e}"))
        })?;
    let circuit_anchors = serde_json::from_str::<lqos_config::CircuitAnchorsFile>(
        &circuit_anchors_json,
    )
    .map_err(|e| PyOSError::new_err(format!("Unable to parse circuit anchor payload JSON: {e}")))?;
    let parent_candidates =
        serde_json::from_str::<lqos_config::TopologyParentCandidatesFile>(&parent_candidates_json)
            .map_err(|e| {
                PyOSError::new_err(format!(
                    "Unable to parse topology parent-candidate payload JSON: {e}"
                ))
            })?;
    let native_editor = serde_json::from_str::<lqos_config::TopologyEditorStateFile>(
        &native_editor_json,
    )
    .map_err(|e| {
        PyOSError::new_err(format!(
            "Unable to parse native topology editor payload JSON: {e}"
        ))
    })?;
    // Rung 1.5 §1.3: the editor payload is no longer a persisted domain
    // object. The legacy Python graph payload still supplies one; project it
    // into canonical (the ONLY persisted object) so compile_mode reads
    // canonical-derived editor state.
    let native_canonical = lqos_config::TopologyCanonicalStateFile::from_editor_and_network(
        &native_editor,
        &compatibility_network_json,
        lqos_config::TopologyCanonicalIngressKind::NativeIntegration,
    );
    let imported = ImportedTopologyBundle {
        source: source.clone(),
        generated_unix: None,
        ingress_identity: None,
        native_canonical: Some(native_canonical),
        parent_candidates: Some(parent_candidates),
        compatibility_network_json,
        shaped_devices,
        circuit_anchors,
        ethernet_advisories: Vec::new(),
        circuit_matches: Default::default(),
    };
    let topology_import = TopologyImportFile::from_imported_bundle(&imported, compile_mode.clone());
    let mode = parse_topology_compile_mode(&compile_mode)?;
    let compiled = compile_topology(imported, mode).map_err(|e| {
        PyOSError::new_err(format!(
            "Unable to compile topology payload for '{source}' using mode '{compile_mode}': {e}"
        ))
    })?;
    let compiled_shaping =
        TopologyCompiledShapingFile::from_compiled(&compiled, compile_mode.clone());
    write_compiled_topology_outputs(
        config.as_ref(),
        &topology_import,
        &compiled_shaping,
        compiled,
    )?;
    Ok(true)
}

#[pyfunction]
fn netzur_api_key() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    Ok(config
        .netzur_integration
        .as_ref()
        .map(|cfg| cfg.api_key.clone())
        .unwrap_or_default())
}

#[pyfunction]
fn netzur_api_url() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    Ok(config
        .netzur_integration
        .as_ref()
        .map(|cfg| cfg.api_url.clone())
        .unwrap_or_default())
}

#[pyfunction]
fn netzur_api_timeout() -> PyResult<u64> {
    let config = lqos_config::load_config().unwrap();
    Ok(config
        .netzur_integration
        .as_ref()
        .map(|cfg| cfg.timeout_secs)
        .unwrap_or(60))
}

#[pyfunction]
fn visp_client_id() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    Ok(config
        .visp_integration
        .as_ref()
        .map(|cfg| cfg.client_id.clone())
        .unwrap_or_default())
}

#[pyfunction]
fn visp_client_secret() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    Ok(config
        .visp_integration
        .as_ref()
        .map(|cfg| cfg.client_secret.clone())
        .unwrap_or_default())
}

#[pyfunction]
fn visp_username() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    Ok(config
        .visp_integration
        .as_ref()
        .map(|cfg| cfg.username.clone())
        .unwrap_or_default())
}

#[pyfunction]
fn visp_password() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    Ok(config
        .visp_integration
        .as_ref()
        .map(|cfg| cfg.password.clone())
        .unwrap_or_default())
}

#[pyfunction]
fn visp_isp_id() -> PyResult<i64> {
    let config = lqos_config::load_config().unwrap();
    Ok(config
        .visp_integration
        .as_ref()
        .and_then(|cfg| cfg.isp_id)
        .unwrap_or(0))
}

#[pyfunction]
fn visp_online_users_domain() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    Ok(config
        .visp_integration
        .as_ref()
        .and_then(|cfg| cfg.online_users_domain.clone())
        .unwrap_or_default())
}

#[pyfunction]
fn visp_timeout_secs() -> PyResult<u64> {
    let config = lqos_config::load_config().unwrap();
    Ok(config
        .visp_integration
        .as_ref()
        .map(|cfg| cfg.timeout_secs)
        .unwrap_or(20))
}

#[pyfunction]
fn automatic_import_uisp() -> PyResult<bool> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.uisp_integration.enable_uisp)
}

#[pyfunction]
fn automatic_import_splynx() -> PyResult<bool> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.splynx_integration.enable_splynx)
}

#[pyfunction]
fn automatic_import_netzur() -> PyResult<bool> {
    let config = lqos_config::load_config().unwrap();
    Ok(config
        .netzur_integration
        .as_ref()
        .map(|cfg| cfg.enable_netzur)
        .unwrap_or(false))
}

#[pyfunction]
fn automatic_import_visp() -> PyResult<bool> {
    let config = lqos_config::load_config().unwrap();
    Ok(config
        .visp_integration
        .as_ref()
        .map(|cfg| cfg.enable_visp)
        .unwrap_or(false))
}

#[pyfunction]
fn queue_refresh_interval_mins() -> PyResult<u32> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.integration_common.queue_refresh_interval_mins)
}

#[pyfunction]
fn automatic_import_powercode() -> PyResult<bool> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.powercode_integration.enable_powercode)
}

#[pyfunction]
fn powercode_api_key() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    let key = config.powercode_integration.powercode_api_key.clone();
    Ok(key)
}

#[pyfunction]
fn powercode_api_url() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    let url = config.powercode_integration.powercode_api_url.clone();
    Ok(url)
}

#[pyfunction]
fn automatic_import_sonar() -> PyResult<bool> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.sonar_integration.enable_sonar)
}

#[pyfunction]
fn sonar_api_url() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    let url = config.sonar_integration.sonar_api_url.clone();
    Ok(url)
}

#[pyfunction]
fn sonar_api_key() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    let key = config.sonar_integration.sonar_api_key.clone();
    Ok(key)
}

#[pyfunction]
fn snmp_community() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    let key = config.sonar_integration.snmp_community.clone();
    Ok(key)
}

#[pyfunction]
fn sonar_airmax_ap_model_ids() -> PyResult<Vec<String>> {
    let config = lqos_config::load_config().unwrap();
    let key = config.sonar_integration.airmax_model_ids.clone();
    Ok(key)
}

#[pyfunction]
fn sonar_ltu_ap_model_ids() -> PyResult<Vec<String>> {
    let config = lqos_config::load_config().unwrap();
    let key = config.sonar_integration.ltu_model_ids.clone();
    Ok(key)
}

#[pyfunction]
fn sonar_active_status_ids() -> PyResult<Vec<String>> {
    let config = lqos_config::load_config().unwrap();
    let key = config.sonar_integration.active_status_ids.clone();
    Ok(key)
}

#[pyfunction]
fn sonar_recurring_service_rates() -> PyResult<Vec<(bool, String, f32, f32)>> {
    let config = lqos_config::load_config().unwrap();
    let rules = config
        .sonar_integration
        .recurring_service_rates
        .iter()
        .map(|rule| {
            (
                rule.enabled,
                rule.service_name.clone(),
                rule.download_mbps,
                rule.upload_mbps,
            )
        })
        .collect();
    Ok(rules)
}

#[pyfunction]
fn sonar_recurring_excluded_service_names() -> PyResult<Vec<String>> {
    let config = lqos_config::load_config().unwrap();
    Ok(config
        .sonar_integration
        .recurring_excluded_service_names
        .clone())
}

#[pyfunction]
fn influx_db_enabled() -> PyResult<bool> {
    let config = lqos_config::load_config().unwrap();
    let Some(config) = config.influxdb.as_ref() else {
        return Ok(false);
    };
    Ok(config.enable_influxdb)
}

#[pyfunction]
fn influx_db_bucket() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    let Some(config) = config.influxdb.as_ref() else {
        return Ok(String::new());
    };
    let bucket = config.bucket.clone();
    Ok(bucket)
}

#[pyfunction]
fn influx_db_org() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    let Some(config) = config.influxdb.as_ref() else {
        return Ok(String::new());
    };
    let org = config.org.clone();
    Ok(org)
}

#[pyfunction]
fn influx_db_token() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    let Some(config) = config.influxdb.as_ref() else {
        return Ok(String::new());
    };
    let token = config.token.clone();
    Ok(token)
}

#[pyfunction]
fn influx_db_url() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    let Some(config) = config.influxdb.as_ref() else {
        return Ok(String::new());
    };
    let url = config.url.clone();
    Ok(url)
}

#[pyfunction]
/// Returns the per-device weighting inputs used by the planner.
pub fn get_weights() -> PyResult<Vec<device_weights::DeviceWeightResponse>> {
    match device_weights::get_weights_rust() {
        Ok(weights) => Ok(weights),
        Err(e) => Err(PyOSError::new_err(e.to_string())),
    }
}

#[pyfunction]
/// Returns calculated tree node weights for the current network graph.
pub fn get_tree_weights() -> PyResult<Vec<device_weights::NetworkNodeWeight>> {
    match device_weights::calculate_tree_weights() {
        Ok(w) => Ok(w),
        Err(e) => Err(PyOSError::new_err(e.to_string())),
    }
}

#[pyfunction]
/// Returns the configured LibreQoS installation directory.
pub fn get_libreqos_directory() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    let dir = config.lqos_directory.clone();
    Ok(dir)
}

#[pyfunction]
/// Returns the configured LibreQoS machine-managed state directory.
pub fn get_libreqos_state_directory() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.resolved_state_directory().display().to_string())
}

#[pyfunction]
/// Returns `true` when the loaded network graph contains only a single root node.
pub fn is_network_flat() -> PyResult<bool> {
    Ok(lqos_network_devices::load_network_json()
        .unwrap()
        .get_nodes_when_ready()
        .len()
        == 1)
}

#[pyfunction]
/// Signals that the current blackboard update batch is complete.
pub fn blackboard_finish() -> PyResult<()> {
    let _ = run_query(vec![BusRequest::BlackboardFinish]);
    Ok(())
}

#[pyfunction]
/// Submits a string value to the selected blackboard namespace.
pub fn blackboard_submit(subsystem: String, key: String, value: String) -> PyResult<()> {
    let subsystem = match subsystem.as_str() {
        "system" => BlackboardSystem::System,
        "site" => BlackboardSystem::Site,
        "circuit" => BlackboardSystem::Circuit,
        _ => return Err(PyOSError::new_err("Invalid subsystem")),
    };
    let _ = run_query(vec![BusRequest::BlackboardData {
        subsystem,
        key,
        value,
    }]);
    Ok(())
}

#[pyfunction]
fn automatic_import_wispgate() -> PyResult<bool> {
    let config = lqos_config::load_config().unwrap();
    let Some(wisp_gate) = config.wispgate_integration.as_ref() else {
        return Ok(false);
    };
    Ok(wisp_gate.enable_wispgate)
}

fn rename_legacy_site_bandwidth_csv_to_backup(file_path: &Path) -> Result<()> {
    let unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let file_name = file_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("legacy_bandwidths.csv");
    let mut backup_path = file_path.with_file_name(format!("{file_name}.backup"));
    if backup_path.exists() {
        backup_path = file_path.with_file_name(format!("{file_name}.backup-{unix_secs}"));
    }
    std::fs::rename(file_path, backup_path)?;
    Ok(())
}

#[pyfunction]
fn migrate_legacy_site_bandwidth_csv(csv_filename: &str) -> PyResult<bool> {
    let trimmed = csv_filename.trim();
    if trimmed.is_empty() {
        return Ok(false);
    }

    let config = lqos_config::load_config().map_err(|e| PyOSError::new_err(e.to_string()))?;
    let legacy_path = Path::new(&config.lqos_directory).join(trimmed);
    if !legacy_path.exists() {
        return Ok(false);
    }

    let mut reader = ReaderBuilder::new()
        .comment(Some(b'#'))
        .trim(csv::Trim::All)
        .from_path(&legacy_path)
        .map_err(|e| {
            PyOSError::new_err(format!("Unable to read {}: {e}", legacy_path.display()))
        })?;

    let operator_path = lqos_overrides::OverrideFile::operator_path_for_config(config.as_ref());
    let mut overrides = if operator_path.exists() {
        lqos_overrides::OverrideFile::load_from_explicit_path(&operator_path).map_err(|e| {
            PyOSError::new_err(format!("Unable to load {}: {e}", operator_path.display()))
        })?
    } else {
        lqos_overrides::OverrideFile::default()
    };

    let mut migrated_rows = 0_usize;
    for (line, record) in reader.records().enumerate() {
        let record = record.map_err(|e| {
            PyOSError::new_err(format!(
                "Unable to decode {} line {}: {e}",
                legacy_path.display(),
                line + 2
            ))
        })?;
        if record.len() != 3 {
            return Err(PyOSError::new_err(format!(
                "Wrong number of records in {} on line {}",
                legacy_path.display(),
                line + 2
            )));
        }

        let site_name = record.get(0).unwrap_or_default().trim().to_string();
        if site_name.is_empty() {
            continue;
        }
        let download = record
            .get(1)
            .unwrap_or_default()
            .trim()
            .parse::<f32>()
            .map_err(|e| {
                PyOSError::new_err(format!(
                    "Unable to parse download bandwidth '{}' in {} on line {}: {e}",
                    record.get(1).unwrap_or_default(),
                    legacy_path.display(),
                    line + 2
                ))
            })?;
        let upload = record
            .get(2)
            .unwrap_or_default()
            .trim()
            .parse::<f32>()
            .map_err(|e| {
                PyOSError::new_err(format!(
                    "Unable to parse upload bandwidth '{}' in {} on line {}: {e}",
                    record.get(2).unwrap_or_default(),
                    legacy_path.display(),
                    line + 2
                ))
            })?;
        let already_present = overrides.network_adjustments().iter().any(|adjustment| {
            matches!(
                adjustment,
                lqos_overrides::NetworkAdjustment::AdjustSiteSpeed { site_name: current_name, .. }
                    if current_name == &site_name
            )
        });
        if already_present {
            continue;
        }
        overrides.set_site_bandwidth_override(None, site_name, Some(download), Some(upload));
        migrated_rows += 1;
    }

    if migrated_rows == 0 {
        return Ok(false);
    }

    overrides
        .save_to_explicit_path(&operator_path)
        .map_err(|e| {
            PyOSError::new_err(format!("Unable to save {}: {e}", operator_path.display()))
        })?;
    rename_legacy_site_bandwidth_csv_to_backup(&legacy_path).map_err(|e| {
        PyOSError::new_err(format!(
            "Unable to rename {} to a backup: {e}",
            legacy_path.display()
        ))
    })?;
    Ok(true)
}

#[pyfunction]
fn wispgate_api_token() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    let Some(wisp_gate) = config.wispgate_integration.as_ref() else {
        return Ok(String::new());
    };
    Ok(wisp_gate.wispgate_api_token.clone())
}

#[pyfunction]
fn wispgate_api_url() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    let Some(wisp_gate) = config.wispgate_integration.as_ref() else {
        return Ok(String::new());
    };
    Ok(wisp_gate.wispgate_api_url.clone())
}

#[pyfunction]
fn promote_to_root_list() -> PyResult<Vec<String>> {
    let config = lqos_config::load_config().unwrap();
    let Some(promote_to_root) = config.integration_common.promote_to_root.as_ref() else {
        return Ok(vec![]);
    };
    Ok(promote_to_root.clone())
}

#[pyfunction]
fn client_bandwidth_multiplier() -> PyResult<f32> {
    let config = lqos_config::load_config().unwrap();
    Ok(config
        .integration_common
        .client_bandwidth_multiplier
        .unwrap_or(1.0))
}
#[pyfunction]
fn enable_insight_topology() -> PyResult<bool> {
    let config = lqos_config::load_config().unwrap();
    Ok(config
        .long_term_stats
        .enable_insight_topology
        .unwrap_or(false))
}

#[pyfunction]
fn insight_topology_role() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    Ok(config
        .long_term_stats
        .insight_topology_role
        .clone()
        .unwrap_or("None".to_string()))
}

#[pyfunction]
fn validated_runtime_shaping_inputs_path() -> PyResult<Option<String>> {
    let Ok(config) = lqos_config::load_config() else {
        return Ok(None);
    };
    lqos_config::validated_runtime_shaping_inputs_path(config.as_ref())
        .map(|path| path.map(|path| path.to_string_lossy().to_string()))
        .map_err(|err| {
            PyOSError::new_err(format!(
                "Unable to validate runtime shaping inputs path: {err}"
            ))
        })
}

/// Verifies the published runtime shaping generation against a fresh dry-run
/// mint of the current world (K1b §4.8 content-addressed generation).
///
/// Returns `None` when the config cannot be loaded. Otherwise returns
/// `(verified, generation, failed_leg)` where `verified` is `None` when the
/// current world produces no shaping inputs (nothing to compare), `Some(true)`
/// when the custody chain holds, and `Some(false)` when the published runtime is
/// stale for the current world. `generation` is the current source-generation
/// token. `failed_leg` names the custody leg that failed (§0b,
/// and is `None` when `verified` is not `Some(false)`.
#[pyfunction]
fn verify_published_shaping_generation() -> PyResult<Option<(Option<bool>, String, Option<String>)>> {
    let Ok(config) = lqos_config::load_config() else {
        return Ok(None);
    };
    let outcome = lqos_topology::verify_published_shaping_generation(config.as_ref()).map_err(
        |err| {
            PyOSError::new_err(format!(
                "Unable to verify the published shaping generation: {err}"
            ))
        },
    )?;
    Ok(Some((
        outcome.verified,
        outcome.generation,
        outcome.failed_leg.map(|leg| leg.as_str().to_string()),
    )))
}

fn shaping_runtime_sqm_fingerprint() -> Result<String> {
    let config = lqos_config::load_config()?;
    let apply_stormguard = config
        .stormguard
        .as_ref()
        .is_some_and(|sg| sg.enabled && !sg.dry_run);
    let overrides =
        lqos_overrides::OverrideStore::load_effective_read_only(apply_stormguard)?;

    let mut sqm_entries: Vec<(String, String)> = overrides
        .circuit_adjustments()
        .iter()
        .filter_map(|adj| match adj {
            lqos_overrides::CircuitAdjustment::DeviceAdjustSqm {
                device_id,
                sqm_override,
            } => {
                let token = sqm_override
                    .as_deref()
                    .map(str::trim)
                    .map(str::to_lowercase)
                    .unwrap_or_default();
                if token.is_empty() {
                    None
                } else {
                    Some((device_id.clone(), token))
                }
            }
            _ => None,
        })
        .collect();
    sqm_entries.sort();

    let mut fingerprint = String::new();
    for (device_id, token) in sqm_entries {
        let _ = writeln!(&mut fingerprint, "{device_id}={token}");
    }

    Ok(fingerprint)
}

fn state_or_legacy_path(
    base_path: &Path,
    state_base_path: &Path,
    category: &str,
    filename: &str,
) -> PathBuf {
    let preferred = state_base_path.join(category).join(filename);
    if preferred.exists() {
        preferred
    } else {
        base_path.join(filename)
    }
}

fn topology_runtime_shaping_generation(base_path: &Path, state_base_path: &Path) -> Option<String> {
    let status_path = state_or_legacy_path(
        base_path,
        state_base_path,
        "topology",
        "topology_runtime_status.json",
    );
    if let Ok(status_raw) = read_to_string(&status_path)
        && let Ok(status) =
            serde_json::from_str::<lqos_config::TopologyRuntimeStatusFile>(&status_raw)
        && status.ready
        && !status.shaping_generation.is_empty()
    {
        return Some(status.shaping_generation);
    }

    let shaping_inputs_path =
        state_or_legacy_path(base_path, state_base_path, "shaping", "shaping_inputs.json");
    let shaping_raw = read_to_string(&shaping_inputs_path).ok()?;
    let shaping_inputs =
        serde_json::from_str::<lqos_config::TopologyShapingInputsFile>(&shaping_raw).ok()?;
    if !shaping_inputs.shaping_generation.is_empty() {
        Some(shaping_inputs.shaping_generation)
    } else {
        shaping_inputs
            .compute_shaping_generation_with_export(None)
            .ok()
    }
}

fn topology_runtime_effective_generation(
    base_path: &Path,
    state_base_path: &Path,
) -> Option<String> {
    let status_path = state_or_legacy_path(
        base_path,
        state_base_path,
        "topology",
        "topology_runtime_status.json",
    );
    if let Ok(status_raw) = read_to_string(&status_path)
        && let Ok(status) =
            serde_json::from_str::<lqos_config::TopologyRuntimeStatusFile>(&status_raw)
        && status.ready
        && !status.effective_generation.is_empty()
    {
        return Some(status.effective_generation);
    }

    // Phase C S5: the E2 operator view is the runtime effective projection;
    // network.effective.json retired. Fold the generation from the same
    // artifact the publish path content-addresses.
    let operator_view_path = state_or_legacy_path(
        base_path,
        state_base_path,
        "topology",
        lqos_config::TOPOLOGY_OPERATOR_VIEW_FILENAME,
    );
    let operator_raw = read_to_string(&operator_view_path).ok()?;
    let operator_view = serde_json::from_str::<serde_json::Value>(&operator_raw).ok()?;
    lqos_config::compute_effective_network_generation(&operator_view).ok()
}

fn calculate_shaping_runtime_hash_for_base(
    base_path: &Path,
    state_base_path: &Path,
    runtime_sqm_fingerprint: &str,
) -> i64 {
    // Phase C S4: the runtime generation artifacts are the ONE hash source;
    // the legacy CSV/interchange hashing retired with the mode.
    let Some(shaping_generation) = topology_runtime_shaping_generation(base_path, state_base_path)
    else {
        return 0;
    };
    let effective_generation =
        topology_runtime_effective_generation(base_path, state_base_path).unwrap_or_default();
    let combined =
        format!("{shaping_generation}\n{effective_generation}\n{runtime_sqm_fingerprint}");
    lqos_utils::hash_to_i64(&combined)
}

#[pyfunction]
fn calculate_shaping_runtime_hash() -> PyResult<i64> {
    let Ok(config) = lqos_config::load_config() else {
        return Ok(0);
    };
    let base_path = Path::new(&config.lqos_directory);
    let state_base_path = config.resolved_state_directory();
    let Ok(runtime_sqm_fingerprint) = shaping_runtime_sqm_fingerprint() else {
        return Ok(0);
    };
    Ok(calculate_shaping_runtime_hash_for_base(
        base_path,
        state_base_path.as_path(),
        &runtime_sqm_fingerprint,
    ))
}

/// Report that the scheduler is still alive
#[pyfunction]
fn scheduler_alive(_py: Python) -> PyResult<bool> {
    if let Ok(reply) = run_query(vec![BusRequest::SchedulerReady]) {
        for resp in reply.iter() {
            if let BusResponse::Ack = resp {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

#[pyfunction]
fn scheduler_error(_py: Python, error: String) -> PyResult<bool> {
    if let Ok(reply) = run_query(vec![BusRequest::SchedulerError(error)]) {
        for resp in reply.iter() {
            if let BusResponse::Ack = resp {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Report informational scheduler output for display in the Web UI.
#[pyfunction]
fn scheduler_output(_py: Python, output: String) -> PyResult<bool> {
    if let Ok(reply) = run_query(vec![BusRequest::SchedulerOutput(output)]) {
        for resp in reply.iter() {
            if let BusResponse::Ack = resp {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Report structured scheduler progress for startup or long-running refresh phases.
#[pyfunction]
fn scheduler_progress(
    _py: Python,
    active: bool,
    phase: String,
    phase_label: String,
    step_index: u32,
    step_count: u32,
    percent: u8,
) -> PyResult<bool> {
    let report = SchedulerProgressReport {
        active,
        phase,
        phase_label,
        step_index,
        step_count,
        percent,
        updated_unix: None,
    };
    if let Ok(reply) = run_query(vec![BusRequest::SchedulerProgress(report)]) {
        for resp in reply.iter() {
            if let BusResponse::Ack = resp {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Report the integration-sync status metadata (the integration health axis).
///
/// `error_class` is classified at emission in `scheduler.py`; the accepted
/// strings are the `IntegrationErrorClass` snake_case names.
#[pyfunction]
#[pyo3(signature = (integration_name=None, last_import_ok=None, last_import_at=None, last_import_ms=None, consecutive_failures=0, error_class=None, error_detail=None, recent_runs=None))]
#[allow(clippy::too_many_arguments)]
fn scheduler_integration_status(
    _py: Python,
    integration_name: Option<String>,
    last_import_ok: Option<bool>,
    last_import_at: Option<u64>,
    last_import_ms: Option<u64>,
    consecutive_failures: u32,
    error_class: Option<String>,
    error_detail: Option<String>,
    recent_runs: Option<Vec<(u64, bool, Option<u64>)>>,
) -> PyResult<bool> {
    let error_class = match error_class.as_deref() {
        Some("redirect_loop") => IntegrationErrorClass::RedirectLoop,
        Some("auth") => IntegrationErrorClass::Auth,
        Some("unreachable") => IntegrationErrorClass::Unreachable,
        Some("exited_unknown") => IntegrationErrorClass::ExitedUnknown,
        _ => IntegrationErrorClass::None,
    };
    let recent_runs = recent_runs
        .unwrap_or_default()
        .into_iter()
        .map(|(at, ok, ms)| SchedulerRun { at, ok, ms })
        .collect();
    let status = SchedulerIntegrationStatus {
        integration_name,
        last_import_ok,
        last_import_at,
        last_import_ms,
        consecutive_failures,
        error_class,
        error_detail,
        recent_runs,
    };
    if let Ok(reply) = run_query(vec![BusRequest::SchedulerIntegrationStatus(status)]) {
        for resp in reply.iter() {
            if let BusResponse::Ack = resp {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Submit an urgent issue for prominent display in the Node Manager UI.
///
/// Parameters:
/// - source: one of "Scheduler", "LibreQoS", "API", "System"
/// - severity: "Error" or "Warning"
/// - code: short machine-readable code (e.g., "TC_U16_OVERFLOW")
/// - message: human-readable description
/// - context: optional JSON string with extra details
/// - dedupe_key: optional key to deduplicate repeats (e.g., code+cpu)
#[pyfunction]
fn submit_urgent_issue(
    _py: Python,
    source: String,
    severity: String,
    code: String,
    message: String,
    context: Option<String>,
    dedupe_key: Option<String>,
) -> PyResult<bool> {
    let src = match source.to_ascii_lowercase().as_str() {
        "scheduler" => UrgentSource::Scheduler,
        "libreqos" => UrgentSource::LibreQoS,
        "api" => UrgentSource::API,
        _ => UrgentSource::System,
    };
    let sev = match severity.to_ascii_lowercase().as_str() {
        "warning" => UrgentSeverity::Warning,
        _ => UrgentSeverity::Error,
    };
    if let Ok(reply) = run_query(vec![BusRequest::SubmitUrgentIssue {
        source: src,
        severity: sev,
        code,
        message,
        context,
        dedupe_key,
    }]) {
        for resp in reply.iter() {
            if let BusResponse::Ack = resp {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Clear urgent issues matching a code and dedupe key.
#[pyfunction]
fn clear_urgent_issue_by_identity(_py: Python, code: String, dedupe_key: String) -> PyResult<bool> {
    if let Ok(reply) = run_query(vec![BusRequest::ClearUrgentIssueByIdentity {
        code: code.clone(),
        dedupe_key: dedupe_key.clone(),
    }]) {
        for resp in reply.iter() {
            if let BusResponse::Ack = resp {
                return Ok(true);
            }
        }
    }

    let Ok(reply) = run_query(vec![BusRequest::GetUrgentIssues]) else {
        return Ok(false);
    };
    let mut ids_to_clear = Vec::new();
    for resp in reply.iter() {
        if let BusResponse::UrgentIssues(issues) = resp {
            ids_to_clear.extend(
                issues
                    .iter()
                    .filter(|issue| {
                        issue.code == code
                            && issue.dedupe_key.as_deref().unwrap_or(&issue.code) == dedupe_key
                    })
                    .map(|issue| issue.id),
            );
        }
    }

    if ids_to_clear.is_empty() {
        return Ok(false);
    }

    let requests = ids_to_clear
        .into_iter()
        .map(BusRequest::ClearUrgentIssue)
        .collect::<Vec<_>>();
    let Ok(reply) = run_query(requests) else {
        return Ok(false);
    };
    Ok(reply.iter().any(|resp| matches!(resp, BusResponse::Ack)))
}

/// Log an informational message via the lqosd bus (appears in lqosd logs).
#[pyfunction]
fn log_info(_py: Python, message: String) -> PyResult<bool> {
    if let Ok(reply) = run_query(vec![BusRequest::LogInfo(message)]) {
        for resp in reply.iter() {
            if let BusResponse::Ack = resp {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

#[pyfunction]
/// Returns whether Insight features are currently enabled in `lqosd`.
pub fn is_insight_enabled() -> PyResult<bool> {
    let Ok(responses) = run_query(vec![BusRequest::GetLtsCapabilities]) else {
        return Ok(false);
    };
    for resp in responses {
        if let BusResponse::LtsCapabilitiesSummary(summary) = resp {
            return Ok(summary.can_view_insight_ui);
        }
    }
    Ok(false)
}

#[pyfunction]
/// Hashes an arbitrary string into the signed 64-bit identifier format used by LibreQoS.
pub fn hash_to_i64(text: String) -> PyResult<i64> {
    Ok(lqos_utils::hash_to_i64(&text))
}

#[cfg(test)]
mod tests {
    use super::{
        calculate_shaping_runtime_hash_for_base, imported_bundle_from_libreqos_network_payload,
    };
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn shaping_runtime_hash_prefers_runtime_artifacts_without_csv() {
        let temp = tempdir().expect("tempdir should exist");
        let base = temp.path();
        fs::write(
            base.join("topology_operator_view.json"),
            r#"{"Root":{"children":{"Site A":{"id":"site-a"}}}}"#,
        )
        .expect("effective network should write");
        fs::write(
            base.join("shaping_inputs.json"),
            r#"{"shaping_generation":"shape-a","circuits":[]}"#,
        )
        .expect("shaping inputs should write");

        let first = calculate_shaping_runtime_hash_for_base(base, base, "");
        assert_ne!(first, 0);

        fs::write(
            base.join("shaping_inputs.json"),
            r#"{"shaping_generation":"shape-b","circuits":[]}"#,
        )
        .expect("updated shaping inputs should write");
        let second = calculate_shaping_runtime_hash_for_base(base, base, "");
        assert_ne!(second, 0);
        assert_ne!(first, second);
    }

    #[test]
    fn shaping_runtime_hash_ignores_runtime_timestamp_only_changes() {
        let temp = tempdir().expect("tempdir should exist");
        let base = temp.path();
        fs::write(
            base.join("shaping_inputs.json"),
            r#"{
                "shaping_generation": "shape-1",
                "generated_unix": 1,
                "circuits": []
            }"#,
        )
        .expect("shaping inputs should write");

        let first = calculate_shaping_runtime_hash_for_base(base, base, "");
        assert_ne!(first, 0);

        fs::write(
            base.join("shaping_inputs.json"),
            r#"{
                "shaping_generation": "shape-1",
                "generated_unix": 2,
                "circuits": []
            }"#,
        )
        .expect("updated shaping inputs should write");

        let second = calculate_shaping_runtime_hash_for_base(base, base, "");
        assert_eq!(first, second);
    }

    #[test]
    fn shaping_runtime_hash_changes_when_effective_network_rates_change() {
        let temp = tempdir().expect("tempdir should exist");
        let base = temp.path();
        fs::write(
            base.join("shaping_inputs.json"),
            r#"{"shaping_generation":"shape-1","circuits":[]}"#,
        )
        .expect("shaping inputs should write");
        fs::write(
            base.join("topology_operator_view.json"),
            r#"{
                "Root": {
                    "children": {
                        "AP A": {
                            "id": "ap-a",
                            "downloadBandwidthMbps": 204,
                            "uploadBandwidthMbps": 58
                        }
                    }
                }
            }"#,
        )
        .expect("effective network should write");

        let first = calculate_shaping_runtime_hash_for_base(base, base, "");
        assert_ne!(first, 0);

        fs::write(
            base.join("topology_operator_view.json"),
            r#"{
                "Root": {
                    "children": {
                        "AP A": {
                            "id": "ap-a",
                            "downloadBandwidthMbps": 218,
                            "uploadBandwidthMbps": 57
                        }
                    }
                }
            }"#,
        )
        .expect("updated effective network should write");

        let second = calculate_shaping_runtime_hash_for_base(base, base, "");
        assert_ne!(second, 0);
        assert_ne!(first, second);
    }

    #[test]
    fn shaping_runtime_hash_ignores_effective_network_key_order_only_changes() {
        let temp = tempdir().expect("tempdir should exist");
        let base = temp.path();
        fs::write(
            base.join("shaping_inputs.json"),
            r#"{"shaping_generation":"shape-1","circuits":[]}"#,
        )
        .expect("shaping inputs should write");
        fs::write(
            base.join("topology_operator_view.json"),
            r#"{
                "Root": {
                    "children": {
                        "AP A": {
                            "id": "ap-a",
                            "downloadBandwidthMbps": 204,
                            "uploadBandwidthMbps": 58
                        }
                    },
                    "downloadBandwidthMbps": 1000,
                    "uploadBandwidthMbps": 1000
                }
            }"#,
        )
        .expect("effective network should write");

        let first = calculate_shaping_runtime_hash_for_base(base, base, "");
        assert_ne!(first, 0);

        fs::write(
            base.join("topology_operator_view.json"),
            r#"{
                "Root": {
                    "uploadBandwidthMbps": 1000,
                    "downloadBandwidthMbps": 1000,
                    "children": {
                        "AP A": {
                            "uploadBandwidthMbps": 58,
                            "downloadBandwidthMbps": 204,
                            "id": "ap-a"
                        }
                    }
                }
            }"#,
        )
        .expect("reordered effective network should write");

        let second = calculate_shaping_runtime_hash_for_base(base, base, "");
        assert_eq!(first, second);
    }

    #[test]
    fn shaping_runtime_hash_uses_resolved_state_directory_when_runtime_files_live_outside_src() {
        let temp = tempdir().expect("tempdir should exist");
        let base = temp.path().join("src");
        let state = temp.path().join("state");
        fs::create_dir_all(state.join("topology")).expect("topology state dir should exist");
        fs::create_dir_all(state.join("shaping")).expect("shaping state dir should exist");

        fs::write(
            state.join("shaping").join("shaping_inputs.json"),
            r#"{"shaping_generation":"shape-1","circuits":[]}"#,
        )
        .expect("shaping inputs should write");
        fs::write(
            state.join("topology").join("topology_operator_view.json"),
            r#"{
                "Root": {
                    "children": {
                        "AP A": {
                            "id": "ap-a",
                            "downloadBandwidthMbps": 204,
                            "uploadBandwidthMbps": 58
                        }
                    }
                }
            }"#,
        )
        .expect("effective network should write");

        let hash = calculate_shaping_runtime_hash_for_base(&base, &state, "");
        assert_ne!(hash, 0);
    }

    #[test]
    fn libreqos_network_payload_door_builds_the_native_bundle() {
        let payload = r#"{
            "schema_version": 1,
            "match": { "mode": "vxlan+vlan" },
            "topology": [ { "id": "s1", "type": "site", "name": "Site 1" } ],
            "circuits": [
                { "id": "c1",
                  "parent": "s1",
                  "plan": { "commit_down_bps": 10000000, "commit_up_bps": 5000000, "ceil_down_bps": 100000000, "ceil_up_bps": 50000000 },
                  "match": { "vni": 100, "vlan": 64 } }
            ]
        }"#;
        let bundle = imported_bundle_from_libreqos_network_payload("libreqos_network", payload)
            .expect("the standard format payload should import");
        let canonical = bundle.native_canonical.expect("native canonical");
        assert_eq!(canonical.nodes.len(), 1);
        assert_eq!(
            bundle.shaped_devices.devices.len(),
            1,
            "a device-less circuit synthesizes one IP-less row so its match survives"
        );
        assert_eq!(
            bundle.circuit_matches.get("c1"),
            Some(&lqos_config::TopologyCircuitMatchIdentity::Tunnel(
                lqos_config::TunnelIdentity {
                    vni: 100,
                    vlan: 64
                }
            ))
        );
    }

    #[test]
    fn libreqos_network_payload_door_rejects_a_stale_compatibility_shape() {
        let error = match imported_bundle_from_libreqos_network_payload(
            "libreqos_network",
            r#"{"Site A":{"children":{}}}"#,
        ) {
            Ok(_) => panic!("the compatibility network.json shape is no longer accepted"),
            Err(error) => error,
        };
        assert!(error.contains("libreqos_network.json"), "{error}");
    }

    /// Upstream #1151 (`lqos-74t.45`): the download accessor returned the UPLINK
    /// cap. Asymmetric guard — `downlink_bandwidth_mbps` -> download,
    /// `uplink_bandwidth_mbps` -> upload — plus the swapped pair so a regression
    /// in either direction fails.
    #[test]
    fn upstream_capacities_map_downlink_to_download_and_uplink_to_upload() {
        let mut config = lqos_config::Config::default();
        // `queues.{downlink,uplink}_bandwidth_mbps` are f64 since the decimal-Mbps
        // change (PR #60); the accessor rounds/saturates to the integer caps.
        config.queues.downlink_bandwidth_mbps = 500.0;
        config.queues.uplink_bandwidth_mbps = 50.0;
        assert_eq!(super::upstream_capacities_mbps(&config), (500, 50));
        config.queues.downlink_bandwidth_mbps = 50.0;
        config.queues.uplink_bandwidth_mbps = 500.0;
        assert_eq!(super::upstream_capacities_mbps(&config), (50, 500));
    }
}
