#![allow(non_local_definitions)] // Temporary: rewrite required for much of this, for newer PyO3.
#![allow(unsafe_op_in_unsafe_fn)]
use lqos_bus::{BlackboardSystem, BusRequest, BusResponse, TcHandle, UrgentSeverity, UrgentSource};
use lqos_utils::hex_string::read_hex_string;
use nix::libc::getpid;
use pyo3::exceptions::PyOSError;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::{
    fs::{File, read_to_string, remove_file},
    io::Write,
    path::Path,
};
mod blocking;
use anyhow::{Error, Result};
use blocking::run_query;
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
    #[serde(default)]
    queuesAvailable: i64,
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

#[pyfunction]
fn write_planner_cbor(py: Python, path: String, state: PyObject) -> PyResult<bool> {
    use std::fs;
    use std::io::Write;
    let dict = state.downcast_bound::<pyo3::types::PyDict>(py)?;
    // Build strongly typed struct, preserving integer keys
    let algo_version = get_string(&dict, "algo_version", default_algo_version());
    let updated_at = get_f64(&dict, "updated_at", 0.0);
    let queues_available = get_i64(&dict, "queuesAvailable", 0);
    let on_a_stick = get_bool(&dict, "on_a_stick", false);
    let site_count = get_i64(&dict, "site_count", 0);
    let mut site_names: Vec<i64> = Vec::new();
    if let Ok(Some(sn)) = dict.get_item("site_names") {
        if let Ok(list) = sn.downcast::<pyo3::types::PyList>() {
            for item in list.iter() {
                if let Some(n) = to_i64_any(&item) {
                    site_names.push(n);
                }
            }
        }
    }
    // site_map
    let mut site_map: BTreeMap<i64, PlannerSiteEntry> = BTreeMap::new();
    if let Ok(Some(sm_any)) = dict.get_item("site_map") {
        if let Ok(sm_dict) = sm_any.downcast::<pyo3::types::PyDict>() {
            for (k, v) in sm_dict.iter() {
                if let Some(key) = to_i64_any(&k) {
                    if let Ok(entry) = v.downcast::<pyo3::types::PyDict>() {
                        let cpu = get_i64(&entry, "cpu", 0);
                        let major = get_i64(&entry, "major", 0);
                        let minor = get_i64(&entry, "minor", 0);
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
        }
    }
    // circuit_map
    let mut circuit_map: BTreeMap<i64, PlannerCircuitEntry> = BTreeMap::new();
    if let Ok(Some(cm_any)) = dict.get_item("circuit_map") {
        if let Ok(cm_dict) = cm_any.downcast::<pyo3::types::PyDict>() {
            for (k, v) in cm_dict.iter() {
                if let Some(key) = to_i64_any(&k) {
                    if let Ok(entry) = v.downcast::<pyo3::types::PyDict>() {
                        let cpu = get_i64(&entry, "cpu", 0);
                        let major = get_i64(&entry, "major", 0);
                        let minor = get_i64(&entry, "minor", 0);
                        let parent_site = get_string(&entry, "parent_site", String::new());
                        let sqm = get_string(&entry, "sqm", String::new());
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
        }
    }

    let to_save = PlannerStateSerde {
        algo_version,
        updated_at,
        queuesAvailable: queues_available,
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
    out.set_item("queuesAvailable", decoded.queuesAvailable)
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
            out.set_item("queuesAvailable", decoded.queuesAvailable)
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
    let algo_version = get_string(&dict, "algo_version", default_algo_version());
    let updated_at = get_f64(&dict, "updated_at", 0.0);
    let queues_available = get_i64(&dict, "queuesAvailable", 0);
    let on_a_stick = get_bool(&dict, "on_a_stick", false);
    let site_count = get_i64(&dict, "site_count", 0);
    // site_names
    let mut site_names: Vec<i64> = Vec::new();
    if let Ok(Some(sn)) = dict.get_item("site_names") {
        if let Ok(list) = sn.downcast::<pyo3::types::PyList>() {
            for item in list.iter() {
                if let Some(n) = to_i64_any(&item) {
                    site_names.push(n);
                }
            }
        }
    }
    // site_map
    let mut site_map: BTreeMap<i64, PlannerSiteEntry> = BTreeMap::new();
    if let Ok(Some(sm_any)) = dict.get_item("site_map") {
        if let Ok(sm_dict) = sm_any.downcast::<pyo3::types::PyDict>() {
            for (k, v) in sm_dict.iter() {
                if let Some(key) = to_i64_any(&k) {
                    if let Ok(entry) = v.downcast::<pyo3::types::PyDict>() {
                        let cpu = get_i64(&entry, "cpu", 0);
                        let major = get_i64(&entry, "major", 0);
                        let minor = get_i64(&entry, "minor", 0);
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
        }
    }
    // circuit_map
    let mut circuit_map: BTreeMap<i64, PlannerCircuitEntry> = BTreeMap::new();
    if let Ok(Some(cm_any)) = dict.get_item("circuit_map") {
        if let Ok(cm_dict) = cm_any.downcast::<pyo3::types::PyDict>() {
            for (k, v) in cm_dict.iter() {
                if let Some(key) = to_i64_any(&k) {
                    if let Ok(entry) = v.downcast::<pyo3::types::PyDict>() {
                        let cpu = get_i64(&entry, "cpu", 0);
                        let major = get_i64(&entry, "major", 0);
                        let minor = get_i64(&entry, "minor", 0);
                        let parent_site = get_string(&entry, "parent_site", String::new());
                        let sqm = get_string(&entry, "sqm", String::new());
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
        }
    }
    let to_save = PlannerStateSerde {
        algo_version,
        updated_at,
        queuesAvailable: queues_available,
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
    m.add_class::<PyIpMapping>()?;
    m.add_class::<BatchedCommands>()?;
    m.add_class::<PyExceptionCpe>()?;
    m.add_class::<device_weights::DeviceWeightResponse>()?;
    m.add_function(wrap_pyfunction!(is_lqosd_alive, m)?)?;
    m.add_function(wrap_pyfunction!(list_ip_mappings, m)?)?;
    m.add_function(wrap_pyfunction!(clear_ip_mappings, m)?)?;
    m.add_function(wrap_pyfunction!(delete_ip_mapping, m)?)?;
    m.add_function(wrap_pyfunction!(add_ip_mapping, m)?)?;
    m.add_function(wrap_pyfunction!(validate_shaped_devices, m)?)?;
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
    m.add_function(wrap_pyfunction!(monitor_mode_only, m)?)?;
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
    m.add_function(wrap_pyfunction!(influx_db_enabled, m)?)?;
    m.add_function(wrap_pyfunction!(influx_db_bucket, m)?)?;
    m.add_function(wrap_pyfunction!(influx_db_org, m)?)?;
    m.add_function(wrap_pyfunction!(influx_db_token, m)?)?;
    m.add_function(wrap_pyfunction!(influx_db_url, m)?)?;
    m.add_function(wrap_pyfunction!(get_weights, m)?)?;
    m.add_function(wrap_pyfunction!(get_tree_weights, m)?)?;
    m.add_function(wrap_pyfunction!(get_libreqos_directory, m)?)?;
    m.add_function(wrap_pyfunction!(overrides_persistent_devices, m)?)?;
    m.add_function(wrap_pyfunction!(overrides_circuit_adjustments, m)?)?;
    m.add_function(wrap_pyfunction!(overrides_network_adjustments, m)?)?;
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
    m.add_function(wrap_pyfunction!(scheduler_alive, m)?)?;
    m.add_function(wrap_pyfunction!(scheduler_error, m)?)?;
    m.add_function(wrap_pyfunction!(submit_urgent_issue, m)?)?;
    m.add_function(wrap_pyfunction!(is_insight_enabled, m)?)?;
    m.add_function(wrap_pyfunction!(log_info, m)?)?;
    m.add_function(wrap_pyfunction!(hash_to_i64, m)?)?;
    // Planner remote fetch/store for Insight integration
    m.add_function(wrap_pyfunction!(fetch_planner_remote, m)?)?;
    m.add_function(wrap_pyfunction!(store_planner_remote, m)?)?;
    m.add_function(wrap_pyfunction!(write_planner_cbor, m)?)?;
    m.add_function(wrap_pyfunction!(read_planner_cbor, m)?)?;

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
    #[pyo3(get)]
    pub ip_address: String,
    #[pyo3(get)]
    pub prefix_length: u32,
    #[pyo3(get)]
    pub tc_handle: (u16, u16),
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
    circuit_id: Option<&str>,
    device_id: Option<&str>,
) -> Result<BusRequest> {
    if !classid.contains(':') {
        return Err(Error::msg(format!(
            "Class id must be in the format (major):(minor), e.g. 1:12. Provided string: {classid}"
        )));
    }
    let circuit_id = circuit_id
        .and_then(|s| (!s.is_empty()).then(|| lqos_utils::hash_to_i64(s) as u64))
        .unwrap_or(0);
    let device_id = device_id
        .and_then(|s| (!s.is_empty()).then(|| lqos_utils::hash_to_i64(s) as u64))
        .unwrap_or(0);
    Ok(BusRequest::MapIpToFlow {
        ip_address: ip.to_string(),
        tc_handle: TcHandle::from_string(classid)?,
        cpu: read_hex_string(cpu)?, // Force HEX representation
        circuit_id,
        device_id,
        upload,
    })
}

/// Adds an IP address mapping
#[pyfunction(signature = (ip, classid, cpu, upload, circuit_id=None, device_id=None))]
fn add_ip_mapping(
    ip: String,
    classid: String,
    cpu: String, // In HEX
    upload: bool,
    circuit_id: Option<String>,
    device_id: Option<String>,
) -> PyResult<()> {
    let request = parse_add_ip(
        &ip,
        &classid,
        &cpu,
        upload,
        circuit_id.as_deref(),
        device_id.as_deref(),
    );
    if let Ok(request) = request {
        run_query(vec![request]).unwrap();
        Ok(())
    } else {
        Err(PyOSError::new_err(request.err().unwrap().to_string()))
    }
}

#[pyclass]
pub struct BatchedCommands {
    batch: Vec<BusRequest>,
}

#[pymethods]
impl BatchedCommands {
    #[new]
    pub fn new() -> PyResult<Self> {
        Ok(Self { batch: Vec::new() })
    }

    #[pyo3(signature = (ip, classid, cpu, upload, circuit_id=None, device_id=None))]
    pub fn add_ip_mapping(
        &mut self,
        ip: String,
        classid: String,
        cpu: String,
        upload: bool,
        circuit_id: Option<String>,
        device_id: Option<String>,
    ) -> PyResult<()> {
        let request = parse_add_ip(
            &ip,
            &classid,
            &cpu,
            upload,
            circuit_id.as_deref(),
            device_id.as_deref(),
        );
        if let Ok(request) = request {
            self.batch.push(request);
            Ok(())
        } else {
            Err(PyOSError::new_err(request.err().unwrap().to_string()))
        }
    }

    pub fn finish_ip_mappings(&mut self) -> PyResult<()> {
        let request = BusRequest::ClearHotCache;
        self.batch.push(request);
        Ok(())
    }

    pub fn length(&self) -> PyResult<usize> {
        Ok(self.batch.len())
    }

    pub fn log(&self) -> PyResult<()> {
        self.batch.iter().for_each(|c| println!("{c:?}"));
        Ok(())
    }

    pub fn submit(&mut self) -> PyResult<usize> {
        const MAX_BATH_SIZE: usize = 512;
        // We're draining the request list out, which is a move that
        // *should* be elided by the optimizing compiler.
        let len = self.batch.len();
        while !self.batch.is_empty() {
            let batch_size = usize::min(MAX_BATH_SIZE, self.batch.len());
            let batch: Vec<BusRequest> = self.batch.drain(0..batch_size).collect();
            run_query(batch).unwrap();
        }
        Ok(len)
    }
}

/// Requests Rust-side validation of `ShapedDevices.csv`
#[pyfunction]
fn validate_shaped_devices() -> PyResult<String> {
    let result = run_query(vec![BusRequest::ValidateShapedDevicesCsv]).unwrap();
    for response in result.iter() {
        match response {
            BusResponse::Ack => return Ok("OK".to_string()),
            BusResponse::ShapedDevicesValidation(error) => return Ok(error.clone()),
            _ => {}
        }
    }
    Ok("".to_string())
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
            lqos_overrides::CircuitAdjustment::CircuitAdjustSpeed { circuit_id, min_download_bandwidth, max_download_bandwidth, min_upload_bandwidth, max_upload_bandwidth } => {
                d.set_item("type", "circuit_adjust_speed")?;
                d.set_item("circuit_id", circuit_id.clone())?;
                if let Some(v) = min_download_bandwidth { d.set_item("min_download_bandwidth", *v)?; }
                if let Some(v) = max_download_bandwidth { d.set_item("max_download_bandwidth", *v)?; }
                if let Some(v) = min_upload_bandwidth { d.set_item("min_upload_bandwidth", *v)?; }
                if let Some(v) = max_upload_bandwidth { d.set_item("max_upload_bandwidth", *v)?; }
            }
            lqos_overrides::CircuitAdjustment::DeviceAdjustSpeed { device_id, min_download_bandwidth, max_download_bandwidth, min_upload_bandwidth, max_upload_bandwidth } => {
                d.set_item("type", "device_adjust_speed")?;
                d.set_item("device_id", device_id.clone())?;
                if let Some(v) = min_download_bandwidth { d.set_item("min_download_bandwidth", *v)?; }
                if let Some(v) = max_download_bandwidth { d.set_item("max_download_bandwidth", *v)?; }
                if let Some(v) = min_upload_bandwidth { d.set_item("min_upload_bandwidth", *v)?; }
                if let Some(v) = max_upload_bandwidth { d.set_item("max_upload_bandwidth", *v)?; }
            }
            lqos_overrides::CircuitAdjustment::RemoveCircuit { circuit_id } => {
                d.set_item("type", "remove_circuit")?;
                d.set_item("circuit_id", circuit_id.clone())?;
            }
            lqos_overrides::CircuitAdjustment::RemoveDevice { device_id } => {
                d.set_item("type", "remove_device")?;
                d.set_item("device_id", device_id.clone())?;
            }
            lqos_overrides::CircuitAdjustment::ReparentCircuit { circuit_id, parent_node } => {
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

    let mut out: Vec<PyObject> = Vec::new();
    for adj in overrides.network_adjustments().iter() {
        let d = PyDict::new(py);
        match adj {
            lqos_overrides::NetworkAdjustment::AdjustSiteSpeed { site_name, download_bandwidth_mbps, upload_bandwidth_mbps } => {
                d.set_item("type", "adjust_site_speed")?;
                d.set_item("site_name", site_name.clone())?;
                if let Some(v) = download_bandwidth_mbps { d.set_item("download_bandwidth_mbps", *v)?; }
                if let Some(v) = upload_bandwidth_mbps { d.set_item("upload_bandwidth_mbps", *v)?; }
            }
            lqos_overrides::NetworkAdjustment::SetNodeVirtual { node_name, virtual_node } => {
                d.set_item("type", "set_node_virtual")?;
                d.set_item("node_name", node_name.clone())?;
                d.set_item("virtual", *virtual_node)?;
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
                if let Some(process) = sys.processes().get(&pid) {
                    if process.name().to_string_lossy().contains("python") {
                        return Ok(true);
                    }
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
fn monitor_mode_only() -> PyResult<bool> {
    let config = lqos_config::load_config().unwrap();
    Ok(config.queues.monitor_only)
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
pub struct PyExceptionCpe {
    pub cpe: String,
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
pub fn get_weights() -> PyResult<Vec<device_weights::DeviceWeightResponse>> {
    match device_weights::get_weights_rust() {
        Ok(weights) => Ok(weights),
        Err(e) => Err(PyOSError::new_err(e.to_string())),
    }
}

#[pyfunction]
pub fn get_tree_weights() -> PyResult<Vec<device_weights::NetworkNodeWeight>> {
    match device_weights::calculate_tree_weights() {
        Ok(w) => Ok(w),
        Err(e) => Err(PyOSError::new_err(e.to_string())),
    }
}

#[pyfunction]
pub fn get_libreqos_directory() -> PyResult<String> {
    let config = lqos_config::load_config().unwrap();
    let dir = config.lqos_directory.clone();
    Ok(dir)
}

#[pyfunction]
pub fn is_network_flat() -> PyResult<bool> {
    Ok(lqos_config::NetworkJson::load()
        .unwrap()
        .get_nodes_when_ready()
        .len()
        == 1)
}

#[pyfunction]
pub fn blackboard_finish() -> PyResult<()> {
    let _ = run_query(vec![BusRequest::BlackboardFinish]);
    Ok(())
}

#[pyfunction]
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

#[pyclass]
pub struct Bakery {
    queue: Vec<BakeryCommands>,
}

#[pymethods]
impl Bakery {
    #[new]
    pub fn new() -> PyResult<Self> {
        Ok(Self { queue: Vec::new() })
    }

    pub fn start_batch(&mut self) -> PyResult<()> {
        self.queue.push(BakeryCommands::StartBatch);
        Ok(())
    }

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
                    let mut bus = lqos_bus::LibreqosBusClient::new().await.unwrap();
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

    pub fn setup_mq(&mut self, queues_available: usize, stick_offset: usize) -> PyResult<()> {
        self.queue.push(BakeryCommands::MqSetup {
            queues_available,
            stick_offset,
        });
        Ok(())
    }

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

    pub fn add_circuit(
        &mut self,
        circuit_name: String,
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

#[pyfunction]
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
pub fn hash_to_i64(text: String) -> PyResult<i64> {
    Ok(lqos_utils::hash_to_i64(&text))
}
