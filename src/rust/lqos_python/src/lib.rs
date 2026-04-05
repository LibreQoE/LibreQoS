//! Python bindings for LibreQoS configuration, queue orchestration, and daemon queries.

#![allow(non_local_definitions)] // Temporary: rewrite required for much of this, for newer PyO3.
#![allow(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]
use lqos_bus::{
    BakeryCapacityReportInterface, BlackboardSystem, BusRequest, BusResponse, TcHandle,
    UrgentSeverity, UrgentSource,
};
use lqos_utils::hex_string::read_hex_string;
use lqos_utils::rustls::ensure_rustls_crypto_provider;
use nix::libc::getpid;
use pyo3::exceptions::PyOSError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use std::fmt::Write as _;
use std::{
    fs::{File, read_to_string, remove_file},
    io::Write,
    path::Path,
};
mod blocking;
use anyhow::{Error, Result};
use blocking::{run_query, run_query_wait_for_bus};
use lqos_bakery::estimate_full_reload_auto_qdisc_budget;
use sysinfo::System;
mod device_weights;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

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
    let items_list = items_any.downcast::<PyList>()?;
    let planner_items: Vec<lqos_config::TopLevelPlannerItem> = items_list
        .iter()
        .map(|item| -> PyResult<lqos_config::TopLevelPlannerItem> {
            let dict = item.downcast::<PyDict>()?;
            let id = get_string(dict, "id", String::new());
            let weight = get_f64(dict, "weight", 1.0);
            Ok(lqos_config::TopLevelPlannerItem { id, weight })
        })
        .collect::<PyResult<Vec<_>>>()?;

    let prev_assign_map = match prev_assign {
        Some(obj) => {
            let any = obj.bind(py);
            let dict = any.downcast::<PyDict>()?;
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
            let dict = any.downcast::<PyDict>()?;
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
    let sites_list = sites.bind(py).downcast::<PyList>()?;
    let site_inputs: Vec<lqos_config::SiteIdentityInput> = sites_list
        .iter()
        .map(|item| -> PyResult<lqos_config::SiteIdentityInput> {
            let dict = item.downcast::<PyDict>()?;
            Ok(lqos_config::SiteIdentityInput {
                site_key: get_string(dict, "site_key", String::new()),
                parent_path: get_string(dict, "parent_path", String::new()),
                queue: get_i64(dict, "queue", 0).max(0) as u32,
                has_children: get_bool(dict, "has_children", false),
            })
        })
        .collect::<PyResult<Vec<_>>>()?;

    let groups_list = circuit_groups.bind(py).downcast::<PyList>()?;
    let circuit_group_inputs: Vec<lqos_config::CircuitIdentityGroupInput> = groups_list
        .iter()
        .map(|item| -> PyResult<lqos_config::CircuitIdentityGroupInput> {
            let dict = item.downcast::<PyDict>()?;
            let ids = match dict.get_item("circuit_ids")? {
                Some(value) => value
                    .downcast::<PyList>()?
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
            let dict = obj.bind(py).downcast::<PyDict>()?;
            dict.iter()
                .filter_map(|(k, v)| {
                    let key = k.extract::<String>().ok()?;
                    let entry = v.downcast::<PyDict>().ok()?;
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
            let dict = obj.bind(py).downcast::<PyDict>()?;
            dict.iter()
                .filter_map(|(k, v)| {
                    let key = k.extract::<String>().ok()?;
                    let entry = v.downcast::<PyDict>().ok()?;
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
    let dict = state.downcast_bound::<pyo3::types::PyDict>(py)?;
    // Build strongly typed struct, preserving integer keys
    let algo_version = get_string(dict, "algo_version", default_algo_version());
    let updated_at = get_f64(dict, "updated_at", 0.0);
    let queues_available = get_i64(dict, "queuesAvailable", 0);
    let on_a_stick = get_bool(dict, "on_a_stick", false);
    let site_count = get_i64(dict, "site_count", 0);
    let mut site_names: Vec<i64> = Vec::new();
    if let Ok(Some(sn)) = dict.get_item("site_names")
        && let Ok(list) = sn.downcast::<pyo3::types::PyList>()
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
        && let Ok(sm_dict) = sm_any.downcast::<pyo3::types::PyDict>()
    {
        for (k, v) in sm_dict.iter() {
            if let Some(key) = to_i64_any(&k)
                && let Ok(entry) = v.downcast::<pyo3::types::PyDict>()
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
        && let Ok(cm_dict) = cm_any.downcast::<pyo3::types::PyDict>()
    {
        for (k, v) in cm_dict.iter() {
            if let Some(key) = to_i64_any(&k)
                && let Ok(entry) = v.downcast::<pyo3::types::PyDict>()
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
    let dict = state.downcast_bound::<pyo3::types::PyDict>(py)?;
    let algo_version = get_string(dict, "algo_version", default_algo_version());
    let updated_at = get_f64(dict, "updated_at", 0.0);
    let queues_available = get_i64(dict, "queuesAvailable", 0);
    let on_a_stick = get_bool(dict, "on_a_stick", false);
    let site_count = get_i64(dict, "site_count", 0);
    // site_names
    let mut site_names: Vec<i64> = Vec::new();
    if let Ok(Some(sn)) = dict.get_item("site_names")
        && let Ok(list) = sn.downcast::<pyo3::types::PyList>()
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
        && let Ok(sm_dict) = sm_any.downcast::<pyo3::types::PyDict>()
    {
        for (k, v) in sm_dict.iter() {
            if let Some(key) = to_i64_any(&k)
                && let Ok(entry) = v.downcast::<pyo3::types::PyDict>()
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
        && let Ok(cm_dict) = cm_any.downcast::<pyo3::types::PyDict>()
    {
        for (k, v) in cm_dict.iter() {
            if let Some(key) = to_i64_any(&k)
                && let Ok(entry) = v.downcast::<pyo3::types::PyDict>()
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

/// Defines the Python module exports.
/// All exported functions have to be listed here.
#[pymodule]
fn liblqos_python(m: &Bound<'_, PyModule>) -> PyResult<()> {
    ensure_rustls_crypto_provider().map_err(|e| PyOSError::new_err(e.to_string()))?;
    m.add_class::<PyIpMapping>()?;
    m.add_class::<BatchedCommands>()?;
    m.add_class::<PyExceptionCpe>()?;
    m.add_class::<device_weights::DeviceWeightResponse>()?;
    m.add_function(wrap_pyfunction!(is_lqosd_alive, m)?)?;
    m.add_function(wrap_pyfunction!(list_ip_mappings, m)?)?;
    m.add_function(wrap_pyfunction!(clear_ip_mappings, m)?)?;
    m.add_function(wrap_pyfunction!(sync_lqosd_config_from_disk, m)?)?;
    m.add_function(wrap_pyfunction!(delete_ip_mapping, m)?)?;
    m.add_function(wrap_pyfunction!(add_ip_mapping, m)?)?;
    m.add_function(wrap_pyfunction!(validate_shaped_devices, m)?)?;
    m.add_function(wrap_pyfunction!(wait_for_bus_ready, m)?)?;
    m.add_function(wrap_pyfunction!(is_libre_already_running, m)?)?;
    m.add_function(wrap_pyfunction!(create_lock_file, m)?)?;
    m.add_function(wrap_pyfunction!(free_lock_file, m)?)?;
    // Unified configuration items
    m.add_function(wrap_pyfunction!(check_config, m)?)?;
    m.add_function(wrap_pyfunction!(sqm, m)?)?;
    m.add_function(wrap_pyfunction!(fast_queues_fq_codel, m)?)?;
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
    m.add_function(wrap_pyfunction!(overwrite_network_json_always, m)?)?;
    m.add_function(wrap_pyfunction!(allowed_subnets, m)?)?;
    m.add_function(wrap_pyfunction!(ignore_subnets, m)?)?;
    m.add_function(wrap_pyfunction!(circuit_name_use_address, m)?)?;
    m.add_function(wrap_pyfunction!(find_ipv6_using_mikrotik, m)?)?;
    m.add_function(wrap_pyfunction!(integration_common_use_mikrotik_ipv6, m)?)?;
    m.add_function(wrap_pyfunction!(exclude_sites, m)?)?;
    m.add_function(wrap_pyfunction!(bandwidth_overhead_factor, m)?)?;
    m.add_function(wrap_pyfunction!(committed_bandwidth_multiplier, m)?)?;
    m.add_function(wrap_pyfunction!(exception_cpes, m)?)?;
    m.add_function(wrap_pyfunction!(uisp_site, m)?)?;
    m.add_function(wrap_pyfunction!(uisp_strategy, m)?)?;
    m.add_function(wrap_pyfunction!(uisp_suspended_strategy, m)?)?;
    m.add_function(wrap_pyfunction!(airmax_capacity, m)?)?;
    m.add_function(wrap_pyfunction!(ltu_capacity, m)?)?;
    m.add_function(wrap_pyfunction!(use_ptmp_as_parent, m)?)?;
    m.add_function(wrap_pyfunction!(uisp_base_url, m)?)?;
    m.add_function(wrap_pyfunction!(uisp_auth_token, m)?)?;
    m.add_function(wrap_pyfunction!(splynx_api_key, m)?)?;
    m.add_function(wrap_pyfunction!(splynx_api_secret, m)?)?;
    m.add_function(wrap_pyfunction!(splynx_api_url, m)?)?;
    m.add_function(wrap_pyfunction!(splynx_strategy, m)?)?;
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
    m.add_function(wrap_pyfunction!(calculate_hash, m)?)?;
    m.add_function(wrap_pyfunction!(calculate_shaping_runtime_hash, m)?)?;
    m.add_function(wrap_pyfunction!(scheduler_alive, m)?)?;
    m.add_function(wrap_pyfunction!(scheduler_error, m)?)?;
    m.add_function(wrap_pyfunction!(scheduler_output, m)?)?;
    m.add_function(wrap_pyfunction!(submit_urgent_issue, m)?)?;
    m.add_function(wrap_pyfunction!(xdp_ip_mapping_capacity, m)?)?;
    m.add_function(wrap_pyfunction!(is_insight_enabled, m)?)?;
    m.add_function(wrap_pyfunction!(log_info, m)?)?;
    m.add_function(wrap_pyfunction!(treeguard_set_node_virtual_live, m)?)?;
    m.add_function(wrap_pyfunction!(treeguard_get_node_virtual_status, m)?)?;
    m.add_function(wrap_pyfunction!(
        treeguard_get_node_virtual_branch_state,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(hash_to_i64, m)?)?;
    // Planner remote fetch/store for Insight integration
    m.add_function(wrap_pyfunction!(fetch_planner_remote, m)?)?;
    m.add_function(wrap_pyfunction!(store_planner_remote, m)?)?;
    m.add_function(wrap_pyfunction!(write_planner_cbor, m)?)?;
    m.add_function(wrap_pyfunction!(read_planner_cbor, m)?)?;
    m.add_function(wrap_pyfunction!(plan_top_level_cpu_bins, m)?)?;
    m.add_function(wrap_pyfunction!(plan_class_identities, m)?)?;

    m.add_class::<Bakery>()?;
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

/// Provides a representation of an IP address mapping
/// Available through python by field name.
#[pyclass]
pub struct PyIpMapping {
    /// IP address or CIDR prefix string assigned to the flow mapping.
    #[pyo3(get)]
    pub ip_address: String,
    /// Prefix length associated with `ip_address`.
    #[pyo3(get)]
    pub prefix_length: u32,
    /// Linux traffic-control handle as `(major, minor)`.
    #[pyo3(get)]
    pub tc_handle: (u16, u16),
    /// CPU index assigned to process traffic for this mapping.
    #[pyo3(get)]
    pub cpu: u32,
}

/// Returns a list of all IP mappings
#[pyfunction]
fn list_ip_mappings(_py: Python) -> PyResult<Vec<PyIpMapping>> {
    let mut result = Vec::new();
    if let Ok(reply) = run_query(vec![BusRequest::ListIpFlow]) {
        for resp in reply.iter() {
            if let BusResponse::MappedIps(map) = resp {
                for mapping in map.iter() {
                    result.push(PyIpMapping {
                        ip_address: mapping.ip_address.clone(),
                        prefix_length: mapping.prefix_length,
                        tc_handle: mapping.tc_handle.get_major_minor(),
                        cpu: mapping.cpu,
                    });
                }
            }
        }
    }
    Ok(result)
}

/// Clear all IP address to TC/CPU mappings
#[pyfunction]
fn clear_ip_mappings(_py: Python) -> PyResult<()> {
    run_query(vec![BusRequest::ClearIpFlow]).unwrap();
    Ok(())
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

/// Deletes an IP to CPU/TC mapping.
///
/// ## Arguments
///
/// * `ip_address`: The IP address to unmap.
/// * `upload`: `true` if this needs to be applied to the upload map (for a split/stick setup)
#[pyfunction]
fn delete_ip_mapping(_py: Python, ip_address: String) -> PyResult<()> {
    run_query(vec![
        BusRequest::DelIpFlow {
            ip_address: ip_address.clone(),
            upload: false,
        },
        BusRequest::DelIpFlow {
            ip_address,
            upload: true,
        },
    ])
    .unwrap();
    Ok(())
}

/// Internal function
/// Converts IP address arguments into an IP mapping request.
fn parse_add_ip(
    ip: &str,
    classid: &str,
    cpu: &str,
    upload: bool,
    circuit_id: &str,
    device_id: &str,
) -> Result<BusRequest> {
    if !classid.contains(':') {
        return Err(Error::msg(format!(
            "Class id must be in the format (major):(minor), e.g. 1:12. Provided string: {classid}"
        )));
    }
    let circuit_id = circuit_id.trim();
    if circuit_id.is_empty() {
        return Err(Error::msg("circuit_id is required"));
    }
    let device_id = device_id.trim();
    if device_id.is_empty() {
        return Err(Error::msg("device_id is required"));
    }
    Ok(BusRequest::MapIpToFlow {
        ip_address: ip.to_string(),
        tc_handle: TcHandle::from_string(classid)?,
        cpu: read_hex_string(cpu)?, // Force HEX representation
        circuit_id: lqos_utils::hash_to_i64(circuit_id) as u64,
        device_id: lqos_utils::hash_to_i64(device_id) as u64,
        upload,
    })
}

/// Adds an IP address mapping
#[pyfunction(signature = (ip, classid, cpu, upload, circuit_id, device_id))]
fn add_ip_mapping(
    ip: String,
    classid: String,
    cpu: String, // In HEX
    upload: bool,
    circuit_id: String,
    device_id: String,
) -> PyResult<()> {
    let request = parse_add_ip(&ip, &classid, &cpu, upload, &circuit_id, &device_id);
    if let Ok(request) = request {
        let responses = run_query(vec![request]).map_err(|e| PyOSError::new_err(e.to_string()))?;
        for response in responses {
            if let BusResponse::Fail(message) = response {
                return Err(PyOSError::new_err(message));
            }
        }
        Ok(())
    } else {
        Err(PyOSError::new_err(request.err().unwrap().to_string()))
    }
}

fn summarize_failure_examples(failures: &BTreeMap<String, usize>) -> String {
    const MAX_EXAMPLES: usize = 3;
    failures
        .iter()
        .take(MAX_EXAMPLES)
        .map(|(message, count)| {
            if *count > 1 {
                format!("{message} (x{count})")
            } else {
                message.clone()
            }
        })
        .collect::<Vec<String>>()
        .join("; ")
}

#[pyclass]
/// Collects IP mapping commands so they can be submitted to `lqosd` in batches.
pub struct BatchedCommands {
    batch: Vec<BusRequest>,
}

#[pymethods]
impl BatchedCommands {
    #[new]
    /// Creates an empty command batch.
    pub fn new() -> PyResult<Self> {
        Ok(Self { batch: Vec::new() })
    }

    #[pyo3(signature = (ip, classid, cpu, upload, circuit_id, device_id))]
    /// Queues an IP-to-flow mapping request for later submission.
    pub fn add_ip_mapping(
        &mut self,
        ip: String,
        classid: String,
        cpu: String,
        upload: bool,
        circuit_id: String,
        device_id: String,
    ) -> PyResult<()> {
        let request = parse_add_ip(&ip, &classid, &cpu, upload, &circuit_id, &device_id);
        if let Ok(request) = request {
            self.batch.push(request);
            Ok(())
        } else {
            Err(PyOSError::new_err(request.err().unwrap().to_string()))
        }
    }

    /// Queues a cache clear after the batch has finished applying mappings.
    pub fn finish_ip_mappings(&mut self) -> PyResult<()> {
        let request = BusRequest::ClearHotCache;
        self.batch.push(request);
        Ok(())
    }

    /// Returns the number of queued requests.
    pub fn length(&self) -> PyResult<usize> {
        Ok(self.batch.len())
    }

    /// Prints queued requests to stdout for debugging.
    pub fn log(&self) -> PyResult<()> {
        self.batch.iter().for_each(|c| println!("{c:?}"));
        Ok(())
    }

    /// Sends queued requests to `lqosd` in chunks and returns how many were submitted.
    pub fn submit(&mut self) -> PyResult<usize> {
        const MAX_BATH_SIZE: usize = 512;
        let len = self.batch.len();
        let mut failed_requests = 0usize;
        let mut failure_examples: BTreeMap<String, usize> = BTreeMap::new();
        while !self.batch.is_empty() {
            let batch_size = usize::min(MAX_BATH_SIZE, self.batch.len());
            let batch: Vec<BusRequest> = self.batch.drain(0..batch_size).collect();
            let responses = run_query(batch).map_err(|e| {
                PyOSError::new_err(format!("IP mapping batch transport failed: {e}"))
            })?;
            responses.into_iter().for_each(|response| {
                if let BusResponse::Fail(message) = response {
                    failed_requests += 1;
                    *failure_examples.entry(message).or_insert(0) += 1;
                }
            });
        }
        if failed_requests > 0 {
            let example_text = summarize_failure_examples(&failure_examples);
            return Err(PyOSError::new_err(format!(
                "IP mapping apply failed for {failed_requests} of {len} queued requests. Examples: {example_text}"
            )));
        }
        Ok(len)
    }
}

/// Returns the current XDP IP-mapping capacity.
///
/// If the live pinned map exists, this reflects its `max_entries`; otherwise it
/// falls back to the compiled default capacity.
#[pyfunction]
fn xdp_ip_mapping_capacity() -> PyResult<usize> {
    Ok(lqos_sys::ip_mapping_capacity())
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
/// circuitID, circuitName, deviceID, deviceName, ParentNode, mac,
/// ipv4s (list[str]), ipv6s (list[str]), minDownload, minUpload, maxDownload,
/// maxUpload, comment, sqm.
#[pyfunction]
fn overrides_persistent_devices(py: Python<'_>) -> PyResult<Vec<PyObject>> {
    let overrides = match lqos_overrides::OverrideFile::load() {
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
    let apply_treeguard = config.treeguard.enabled;

    let overrides =
        match lqos_overrides::OverrideStore::load_effective(apply_stormguard, apply_treeguard) {
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
    let overrides = match lqos_overrides::OverrideStore::load_effective(false, false) {
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
    let overrides = match lqos_overrides::OverrideFile::load() {
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
    let apply_treeguard = config.treeguard.enabled;

    let overrides =
        match lqos_overrides::OverrideStore::load_effective(apply_stormguard, apply_treeguard) {
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
    let overrides = match lqos_overrides::OverrideStore::load_effective(false, false) {
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
    let overrides = match lqos_overrides::OverrideFile::load() {
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
    let apply_treeguard = config.treeguard.enabled;

    let overrides =
        match lqos_overrides::OverrideStore::load_effective(apply_stormguard, apply_treeguard) {
            Ok(o) => o,
            Err(e) => return Err(PyOSError::new_err(e.to_string())),
        };

    network_adjustments_to_py(py, overrides.network_adjustments())
}

/// Returns the list of network adjustments that should be materialized into `network.json`.
///
/// This includes only operator-owned network adjustments.
///
/// TreeGuard virtual-node decisions and StormGuard adaptive site-speed decisions are intentionally
/// excluded so runtime automation does not overwrite the operator-authored topology/source-of-truth
/// file.
#[pyfunction]
fn overrides_network_adjustments_materialized(py: Python<'_>) -> PyResult<Vec<PyObject>> {
    let overrides = match lqos_overrides::OverrideStore::load_effective(false, false) {
        Ok(o) => o,
        Err(e) => return Err(PyOSError::new_err(e.to_string())),
    };

    network_adjustments_to_py(py, overrides.network_adjustments())
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
            }
            lqos_overrides::NetworkAdjustment::SetNodeVirtual {
                node_name,
                virtual_node,
            } => {
                d.set_item("type", "set_node_virtual")?;
                d.set_item("node_name", node_name.clone())?;
                d.set_item("virtual", *virtual_node)?;
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

/// Returns the Mbps threshold at or above which (if no per-circuit override is set)
/// fq_codel should be preferred to reduce overhead on very fast circuits.
/// Defaults to 1000.0 Mbps if not configured.
#[pyfunction]
fn fast_queues_fq_codel() -> PyResult<f32> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.queues.fast_queues_fq_codel.unwrap_or(1000.0) as f32)
}

#[pyfunction]
fn upstream_bandwidth_capacity_download_mbps() -> PyResult<u32> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.queues.uplink_bandwidth_mbps as u32)
}

#[pyfunction]
fn upstream_bandwidth_capacity_upload_mbps() -> PyResult<u32> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.queues.uplink_bandwidth_mbps as u32)
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
    let config = lqos_config::load_config().unwrap();
    Ok(config.on_a_stick_mode())
}

#[pyfunction]
fn overwrite_network_json_always() -> PyResult<bool> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.integration_common.always_overwrite_network_json)
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
    pub cpe: String,
    /// Parent identifier assigned to the exception CPE.
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
    let strategy = config.uisp_integration.strategy.clone();
    Ok(strategy)
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
fn use_ptmp_as_parent() -> PyResult<bool> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.uisp_integration.use_ptmp_as_parent)
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
        Ok(config) => Ok(config.splynx_integration.strategy.clone()),
        Err(_) => Ok("ap_only".to_string()), // Default value when config can't be loaded
    }
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
/// Returns `true` when the loaded network graph contains only a single root node.
pub fn is_network_flat() -> PyResult<bool> {
    Ok(lqos_config::NetworkJson::load()
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
fn calculate_hash() -> PyResult<i64> {
    let Ok(config) = lqos_config::load_config() else {
        return Ok(0);
    };
    let nj_path = Path::new(&config.lqos_directory).join("network.json");
    let sd_path = Path::new(&config.lqos_directory).join("ShapedDevices.csv");

    let Ok(nj_as_string) = read_to_string(nj_path) else {
        return Ok(0);
    };
    let Ok(sd_as_string) = read_to_string(sd_path) else {
        return Ok(0);
    };
    let combined = format!("{}\n{}", nj_as_string, sd_as_string);
    let hash = lqos_utils::hash_to_i64(&combined);

    Ok(hash)
}

fn shaping_runtime_sqm_fingerprint() -> Result<String> {
    let config = lqos_config::load_config()?;
    let apply_stormguard = config
        .stormguard
        .as_ref()
        .is_some_and(|sg| sg.enabled && !sg.dry_run);
    let apply_treeguard = config.treeguard.enabled;
    let overrides =
        lqos_overrides::OverrideStore::load_effective(apply_stormguard, apply_treeguard)?;

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

#[pyfunction]
fn calculate_shaping_runtime_hash() -> PyResult<i64> {
    let Ok(config) = lqos_config::load_config() else {
        return Ok(0);
    };
    let base_path = Path::new(&config.lqos_directory);
    let effective_path = base_path.join("network.effective.json");
    let nj_path = if effective_path.exists() {
        effective_path
    } else if config.long_term_stats.enable_insight_topology.unwrap_or(false) {
        let insight_path = base_path.join("network.insight.json");
        if insight_path.exists() {
            insight_path
        } else {
            base_path.join("network.json")
        }
    } else {
        base_path.join("network.json")
    };
    let sd_path = if config.long_term_stats.enable_insight_topology.unwrap_or(false) {
        let insight_path = base_path.join("ShapedDevices.insight.csv");
        if insight_path.exists() {
            insight_path
        } else {
            base_path.join("ShapedDevices.csv")
        }
    } else {
        base_path.join("ShapedDevices.csv")
    };

    let Ok(nj_as_string) = read_to_string(nj_path) else {
        return Ok(0);
    };
    let Ok(sd_as_string) = read_to_string(sd_path) else {
        return Ok(0);
    };
    let Ok(runtime_sqm_fingerprint) = shaping_runtime_sqm_fingerprint() else {
        return Ok(0);
    };

    let combined = format!(
        "{}\n{}\n{}",
        nj_as_string, sd_as_string, runtime_sqm_fingerprint
    );
    Ok(lqos_utils::hash_to_i64(&combined))
}

////////////////////////////// The Bakery class //////////////////////////////

#[derive(Clone)]
enum BakeryCommands {
    StartBatch,
    Commit,
    MqSetup {
        queues_available: usize,
        stick_offset: usize,
    },
    AddSite {
        site_hash: i64,
        parent_class_id: TcHandle,
        up_parent_class_id: TcHandle,
        class_minor: u16,
        download_bandwidth_min: f32,
        upload_bandwidth_min: f32,
        download_bandwidth_max: f32,
        upload_bandwidth_max: f32,
    },
    AddCircuit {
        circuit_hash: i64,
        circuit_name: Option<String>,
        site_name: Option<String>,
        parent_class_id: TcHandle,
        up_parent_class_id: TcHandle,
        class_minor: u16,
        download_bandwidth_min: f32,
        upload_bandwidth_min: f32,
        download_bandwidth_max: f32,
        upload_bandwidth_max: f32,
        class_major: u16,
        up_class_major: u16,
        ip_addresses: String,
        sqm_override: Option<String>,
    },
}

impl BakeryCommands {
    fn as_runtime_command(&self) -> lqos_bakery::BakeryCommands {
        match self {
            BakeryCommands::StartBatch => lqos_bakery::BakeryCommands::StartBatch,
            BakeryCommands::Commit => lqos_bakery::BakeryCommands::CommitBatch,
            BakeryCommands::MqSetup {
                queues_available,
                stick_offset,
            } => lqos_bakery::BakeryCommands::MqSetup {
                queues_available: *queues_available,
                stick_offset: *stick_offset,
            },
            BakeryCommands::AddSite {
                site_hash,
                parent_class_id,
                up_parent_class_id,
                class_minor,
                download_bandwidth_min,
                upload_bandwidth_min,
                download_bandwidth_max,
                upload_bandwidth_max,
            } => lqos_bakery::BakeryCommands::AddSite {
                site_hash: *site_hash,
                parent_class_id: *parent_class_id,
                up_parent_class_id: *up_parent_class_id,
                class_minor: *class_minor,
                download_bandwidth_min: *download_bandwidth_min,
                upload_bandwidth_min: *upload_bandwidth_min,
                download_bandwidth_max: *download_bandwidth_max,
                upload_bandwidth_max: *upload_bandwidth_max,
            },
            BakeryCommands::AddCircuit {
                circuit_hash,
                circuit_name,
                site_name,
                parent_class_id,
                up_parent_class_id,
                class_minor,
                download_bandwidth_min,
                upload_bandwidth_min,
                download_bandwidth_max,
                upload_bandwidth_max,
                class_major,
                up_class_major,
                ip_addresses,
                sqm_override,
            } => lqos_bakery::BakeryCommands::AddCircuit {
                circuit_hash: *circuit_hash,
                circuit_name: circuit_name.clone(),
                site_name: site_name.clone(),
                parent_class_id: *parent_class_id,
                up_parent_class_id: *up_parent_class_id,
                class_minor: *class_minor,
                download_bandwidth_min: *download_bandwidth_min,
                upload_bandwidth_min: *upload_bandwidth_min,
                download_bandwidth_max: *download_bandwidth_max,
                upload_bandwidth_max: *upload_bandwidth_max,
                class_major: *class_major,
                up_class_major: *up_class_major,
                down_qdisc_handle: None,
                up_qdisc_handle: None,
                ip_addresses: ip_addresses.clone(),
                sqm_override: sqm_override.clone(),
            },
        }
    }
}

#[pyclass]
/// Queues Bakery operations for batched submission to the LibreQoS daemon.
pub struct Bakery {
    queue: Vec<BakeryCommands>,
}

#[pymethods]
impl Bakery {
    #[new]
    /// Creates an empty Bakery command queue.
    pub fn new() -> PyResult<Self> {
        Ok(Self { queue: Vec::new() })
    }

    /// Adds a batch-start marker to the queued Bakery commands.
    pub fn start_batch(&mut self) -> PyResult<()> {
        self.queue.push(BakeryCommands::StartBatch);
        Ok(())
    }

    /// Sends the queued Bakery commands to `lqosd`.
    pub fn commit(&mut self) -> PyResult<()> {
        self.queue.push(BakeryCommands::Commit);

        // Send the commands batched up to the bus
        let queue = self.queue.clone();
        let handle = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let Ok(mut bus) = lqos_bus::LibreqosBusClient::new().await else {
                        eprintln!("Failed to connect to lqosd bus for Bakery commit");
                        return;
                    };
                    let chunks = queue.chunks(1024);
                    for chunk in chunks {
                        let mut requests = Vec::new();
                        for msg in chunk {
                            match msg {
                                BakeryCommands::StartBatch => {
                                    requests.push(BusRequest::BakeryStart)
                                }
                                BakeryCommands::Commit => requests.push(BusRequest::BakeryCommit),
                                BakeryCommands::MqSetup {
                                    queues_available,
                                    stick_offset,
                                } => {
                                    requests.push(BusRequest::BakeryMqSetup {
                                        queues_available: *queues_available,
                                        stick_offset: *stick_offset,
                                    });
                                }
                                BakeryCommands::AddSite {
                                    site_hash,
                                    parent_class_id,
                                    up_parent_class_id,
                                    class_minor,
                                    download_bandwidth_min,
                                    upload_bandwidth_min,
                                    download_bandwidth_max,
                                    upload_bandwidth_max,
                                } => {
                                    let command = BusRequest::BakeryAddSite {
                                        site_hash: *site_hash,
                                        parent_class_id: *parent_class_id,
                                        up_parent_class_id: *up_parent_class_id,
                                        class_minor: *class_minor,
                                        download_bandwidth_min: *download_bandwidth_min,
                                        upload_bandwidth_min: *upload_bandwidth_min,
                                        download_bandwidth_max: *download_bandwidth_max,
                                        upload_bandwidth_max: *upload_bandwidth_max,
                                    };
                                    requests.push(command);
                                }
                                BakeryCommands::AddCircuit {
                                    circuit_hash,
                                    circuit_name,
                                    site_name,
                                    parent_class_id,
                                    up_parent_class_id,
                                    class_minor,
                                    download_bandwidth_min,
                                    upload_bandwidth_min,
                                    download_bandwidth_max,
                                    upload_bandwidth_max,
                                    class_major,
                                    up_class_major,
                                    ip_addresses,
                                    sqm_override,
                                } => {
                                    let command = BusRequest::BakeryAddCircuit {
                                        circuit_hash: *circuit_hash,
                                        circuit_name: circuit_name.clone(),
                                        site_name: site_name.clone(),
                                        parent_class_id: *parent_class_id,
                                        up_parent_class_id: *up_parent_class_id,
                                        class_minor: *class_minor,
                                        download_bandwidth_min: *download_bandwidth_min,
                                        upload_bandwidth_min: *upload_bandwidth_min,
                                        download_bandwidth_max: *download_bandwidth_max,
                                        upload_bandwidth_max: *upload_bandwidth_max,
                                        class_major: *class_major,
                                        up_class_major: *up_class_major,
                                        ip_addresses: ip_addresses.clone(),
                                        sqm_override: sqm_override.clone(),
                                    };
                                    requests.push(command);
                                }
                            }
                        }
                        if let Err(e) = bus.request(requests).await {
                            eprintln!("Failed to send batch commands: {}", e);
                        } else {
                            println!("Sent a batch of commands to Bakery");
                        }
                    }
                });
        });
        let _ = handle.join();

        Ok(())
    }

    /// Estimates whether the queued full-reload batch fits within the per-interface qdisc budget.
    pub fn estimate_qdisc_budget(&self, py: Python) -> PyResult<PyObject> {
        let config = lqos_config::load_config().map_err(|e| PyOSError::new_err(e.to_string()))?;
        let queue: Vec<lqos_bakery::BakeryCommands> = self
            .queue
            .iter()
            .map(BakeryCommands::as_runtime_command)
            .collect();
        let estimate = estimate_full_reload_auto_qdisc_budget(&config, &queue);
        let is_ok = estimate.ok();
        let interface_reports = estimate
            .interface_details
            .iter()
            .map(|(name, detail)| BakeryCapacityReportInterface {
                name: name.clone(),
                planned_qdiscs: detail.planned_qdiscs,
                infra_qdiscs: detail.infra_qdiscs,
                cake_qdiscs: detail.cake_qdiscs,
                fq_codel_qdiscs: detail.fq_codel_qdiscs,
                estimated_memory_bytes: detail.estimated_memory_bytes,
            })
            .collect::<Vec<_>>();
        let memory_summary = if let Some(snapshot) = estimate.memory_snapshot.as_ref() {
            format!(
                "estimated qdisc memory {} bytes with {} bytes currently available and safety floor {} bytes",
                estimate.estimated_total_memory_bytes,
                snapshot.available_bytes,
                lqos_bakery::BAKERY_MEMORY_GUARD_MIN_AVAILABLE_BYTES
            )
        } else {
            format!(
                "estimated qdisc memory {} bytes; host memory snapshot unavailable",
                estimate.estimated_total_memory_bytes
            )
        };
        let summary = if interface_reports.is_empty() {
            format!(
                "Planned queue model {} preflight. No shaping interfaces were queued; {memory_summary}.",
                if is_ok { "fits" } else { "exceeds" },
            )
        } else {
            let interface_summary = interface_reports
                .iter()
                .map(|entry| {
                    let detail = estimate
                        .interface_details
                        .get(&entry.name)
                        .expect("detail should exist for interface");
                    format!(
                        "{} estimated {} qdiscs (infra {}, cake {}, fq_codel {})",
                        entry.name,
                        entry.planned_qdiscs,
                        detail.infra_qdiscs,
                        detail.cake_qdiscs,
                        detail.fq_codel_qdiscs
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "Planned queue model {} preflight. {interface_summary}; safe budget {}, kernel limit {}; {memory_summary}.",
                if is_ok { "fits" } else { "exceeds" },
                estimate.safe_budget,
                estimate.hard_limit,
            )
        };
        let _ = run_query(vec![BusRequest::BakeryReportPreflight {
            ok: is_ok,
            message: summary.clone(),
            safe_budget: estimate.safe_budget,
            hard_limit: estimate.hard_limit,
            estimated_total_memory_bytes: estimate.estimated_total_memory_bytes,
            memory_available_bytes: estimate
                .memory_snapshot
                .as_ref()
                .map(|snapshot| snapshot.available_bytes),
            memory_guard_min_available_bytes: lqos_bakery::BAKERY_MEMORY_GUARD_MIN_AVAILABLE_BYTES,
            memory_ok: estimate.memory_ok,
            interfaces: interface_reports,
        }]);

        let result = PyDict::new(py);
        let interfaces = PyDict::new(py);
        let interface_details = PyDict::new(py);

        for (interface, count) in estimate.interfaces {
            interfaces.set_item(interface, count)?;
        }
        for (interface, detail) in estimate.interface_details {
            let detail_dict = PyDict::new(py);
            detail_dict.set_item("planned_qdiscs", detail.planned_qdiscs)?;
            detail_dict.set_item("infra_qdiscs", detail.infra_qdiscs)?;
            detail_dict.set_item("cake_qdiscs", detail.cake_qdiscs)?;
            detail_dict.set_item("fq_codel_qdiscs", detail.fq_codel_qdiscs)?;
            detail_dict.set_item("estimated_memory_bytes", detail.estimated_memory_bytes)?;
            interface_details.set_item(interface, detail_dict)?;
        }

        result.set_item("interfaces", interfaces)?;
        result.set_item("interface_details", interface_details)?;
        result.set_item("safe_budget", estimate.safe_budget)?;
        result.set_item("hard_limit", estimate.hard_limit)?;
        result.set_item(
            "estimated_total_memory_bytes",
            estimate.estimated_total_memory_bytes,
        )?;
        result.set_item("memory_ok", estimate.memory_ok)?;
        result.set_item(
            "memory_guard_min_available_bytes",
            lqos_bakery::BAKERY_MEMORY_GUARD_MIN_AVAILABLE_BYTES,
        )?;
        if let Some(snapshot) = estimate.memory_snapshot {
            result.set_item("memory_total_bytes", snapshot.total_bytes)?;
            result.set_item("memory_available_bytes", snapshot.available_bytes)?;
        } else {
            result.set_item("memory_total_bytes", py.None())?;
            result.set_item("memory_available_bytes", py.None())?;
        }
        result.set_item("ok", is_ok)?;
        result.set_item("summary", summary)?;
        Ok(result.into())
    }

    /// Queues multi-queue setup for the target shaper.
    pub fn setup_mq(&mut self, queues_available: usize, stick_offset: usize) -> PyResult<()> {
        self.queue.push(BakeryCommands::MqSetup {
            queues_available,
            stick_offset,
        });
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    /// Queues a site creation or update command for the Bakery pipeline.
    pub fn add_site(
        &mut self,
        site_name: String,
        parent_class_id: String,
        up_parent_class_id: String,
        class_minor: u16,
        download_bandwidth_min: f32,
        upload_bandwidth_min: f32,
        download_bandwidth_max: f32,
        upload_bandwidth_max: f32,
    ) -> PyResult<()> {
        let site_hash = lqos_utils::hash_to_i64(&site_name);
        //println!("Name hash for site {site_name} is {site_hash}");
        let command = BakeryCommands::AddSite {
            site_hash,
            parent_class_id: TcHandle::from_string(&parent_class_id).unwrap(),
            up_parent_class_id: TcHandle::from_string(&up_parent_class_id).unwrap(),
            class_minor,
            download_bandwidth_min,
            upload_bandwidth_min,
            download_bandwidth_max,
            upload_bandwidth_max,
        };
        self.queue.push(command);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    /// Queues a circuit creation or update command for the Bakery pipeline.
    pub fn add_circuit(
        &mut self,
        circuit_name: String,
        site_name: Option<String>,
        parent_class_id: String,
        up_parent_class_id: String,
        class_minor: u16,
        download_bandwidth_min: f32,
        upload_bandwidth_min: f32,
        download_bandwidth_max: f32,
        upload_bandwidth_max: f32,
        class_major: u16,
        up_class_major: u16,
        ip_addresses: String,
        sqm_override: Option<String>,
    ) -> PyResult<()> {
        let circuit_hash = lqos_utils::hash_to_i64(&circuit_name);
        //println!("Name: {circuit_name}, hash: {circuit_hash}");
        let command = BakeryCommands::AddCircuit {
            circuit_hash,
            circuit_name: Some(circuit_name),
            site_name,
            parent_class_id: TcHandle::from_string(&parent_class_id).unwrap(),
            up_parent_class_id: TcHandle::from_string(&up_parent_class_id).unwrap(),
            class_minor,
            download_bandwidth_min,
            upload_bandwidth_min,
            download_bandwidth_max,
            upload_bandwidth_max,
            class_major,
            up_class_major,
            ip_addresses,
            sqm_override,
        };
        self.queue.push(command);
        Ok(())
    }
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

/// Submit a live Bakery runtime node virtualization or restore intent for a named node.
#[pyfunction]
fn treeguard_set_node_virtual_live(
    _py: Python,
    node_name: String,
    virtualized: bool,
) -> PyResult<bool> {
    if let Ok(reply) = run_query(vec![BusRequest::TreeGuardSetNodeVirtual {
        node_name,
        virtualized,
    }]) {
        for resp in reply.iter() {
            if let BusResponse::Ack = resp {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Fetch the latest Bakery runtime node-operation snapshot for a named node, if any.
#[pyfunction]
fn treeguard_get_node_virtual_status(
    py: Python,
    node_name: String,
) -> PyResult<Option<Py<PyDict>>> {
    let Ok(reply) = run_query(vec![BusRequest::TreeGuardGetNodeVirtualStatus {
        node_name,
    }]) else {
        return Ok(None);
    };
    for resp in reply {
        if let BusResponse::TreeGuardRuntimeNodeOperation(snapshot) = resp {
            let Some(snapshot) = snapshot else {
                return Ok(None);
            };
            let failure_reason = snapshot.failure_reason.clone();
            let d = PyDict::new(py);
            d.set_item("operation_id", snapshot.operation_id)?;
            d.set_item("site_hash", snapshot.site_hash)?;
            d.set_item("action", snapshot.action)?;
            d.set_item("status", snapshot.status)?;
            d.set_item("attempt_count", snapshot.attempt_count)?;
            d.set_item("submitted_at_unix", snapshot.submitted_at_unix)?;
            d.set_item("updated_at_unix", snapshot.updated_at_unix)?;
            d.set_item("next_retry_at_unix", snapshot.next_retry_at_unix)?;
            d.set_item("last_error", snapshot.last_error)?;
            d.set_item("failure_reason", failure_reason)?;
            return Ok(Some(d.unbind()));
        }
    }
    Ok(None)
}

/// Fetch the latest Bakery runtime branch-state snapshot for a named node, if one is retained.
#[pyfunction]
fn treeguard_get_node_virtual_branch_state(
    py: Python,
    node_name: String,
) -> PyResult<Option<Py<PyDict>>> {
    let Ok(reply) = run_query(vec![BusRequest::TreeGuardGetNodeVirtualBranchState {
        node_name,
    }]) else {
        return Ok(None);
    };
    for resp in reply {
        if let BusResponse::TreeGuardRuntimeNodeBranch(snapshot) = resp {
            let Some(snapshot) = snapshot else {
                return Ok(None);
            };
            let d = PyDict::new(py);
            d.set_item("site_hash", snapshot.site_hash)?;
            d.set_item("active_branch", snapshot.active_branch)?;
            d.set_item("lifecycle", snapshot.lifecycle)?;
            d.set_item("pending_prune", snapshot.pending_prune)?;
            d.set_item("next_prune_attempt_unix", snapshot.next_prune_attempt_unix)?;
            d.set_item("active_site_hashes", snapshot.active_site_hashes)?;
            d.set_item("saved_site_hashes", snapshot.saved_site_hashes)?;
            d.set_item("prune_site_hashes", snapshot.prune_site_hashes)?;
            d.set_item("qdisc_down_major", snapshot.qdisc_down_major)?;
            d.set_item("qdisc_up_major", snapshot.qdisc_up_major)?;
            return Ok(Some(d.unbind()));
        }
    }
    Ok(None)
}

#[pyfunction]
/// Returns whether Insight features are currently enabled in `lqosd`.
pub fn is_insight_enabled() -> PyResult<bool> {
    let Ok(responses) = run_query(vec![BusRequest::CheckInsight]) else {
        return Ok(false);
    };
    for resp in responses {
        if let BusResponse::InsightStatus(enabled) = resp {
            return Ok(enabled);
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
    use super::summarize_failure_examples;
    use std::collections::BTreeMap;

    #[test]
    fn summarize_failure_examples_limits_output() {
        let mut failures = BTreeMap::new();
        failures.insert("alpha".to_string(), 2);
        failures.insert("beta".to_string(), 1);
        failures.insert("delta".to_string(), 1);
        failures.insert("gamma".to_string(), 1);

        assert_eq!(
            summarize_failure_examples(&failures),
            "alpha (x2); beta; delta"
        );
    }
}
