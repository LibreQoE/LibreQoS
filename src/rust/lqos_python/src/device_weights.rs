//! This module is responsible for getting the device weights from the Long Term Stats API
//! and the ShapedDevices.csv file. It will merge the two sources of weights and return the
//! result as a Vec<DeviceWeightResponse>.
//!
//! # Example
//!
//! ```python
//! from liblqos_python import get_weights;
//! weights = get_weights();
//! for w in weights:
//!    print(w.circuit_id + " : " + str(w.weight));
//! ```

use anyhow::Result;
use lqos_bus::{BusRequest, BusResponse};
use lqos_config::{ConfigShapedDevices, ShapedDevice, load_config};
use pyo3::pyclass;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// This struct is used to send a request to the Long Term Stats API
#[derive(Serialize, Deserialize)]
pub struct DeviceWeightRequest {
    pub org_key: String,
    pub node_id: String,
    pub start: i64,
    pub duration_seconds: i64,
    pub percentile: f64,
}

/// This struct is used to receive a response from the Long Term Stats API
/// It contains the circuit_id and the weight of the device
#[derive(Serialize, Deserialize, Debug)]
#[pyclass]
pub struct DeviceWeightResponse {
    #[pyo3(get)]
    pub circuit_id: String,
    #[pyo3(get)]
    pub weight: i64,
}

/// This function is used to get the device weights from the Long Term Stats API
fn get_weights_from_lts(
    org_key: &str,
    node_id: &str,
    start: i64,
    duration_seconds: i64,
    percentile: f64,
) -> Result<Vec<DeviceWeightResponse>> {
    let request = DeviceWeightRequest {
        org_key: org_key.to_string(),
        node_id: node_id.to_string(),
        start,
        duration_seconds,
        percentile,
    };

    // Build the URL
    let config = load_config()?;
    let base_url = config
        .long_term_stats
        .lts_url
        .clone()
        .unwrap_or("insight.libreqos.com".to_string());
    let url = format!("https://{}/shaper_api/deviceWeights", base_url);

    // Make a BLOCKING reqwest call (we're not in an async context)
    // Allow invalid certs for self-hosted Insight instances (non-default URL)
    let allow_insecure = !url.starts_with("https://insight.libreqos.com");
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(6))
        .danger_accept_invalid_certs(allow_insecure)
        .build()?;

    let response = client
        .post(url)
        .json(&request)
        .send()?
        .json::<Vec<DeviceWeightResponse>>()?;

    Ok(response)
}

/// This function is used to get the device weights from the ShapedDevices.csv file
fn get_weights_from_shaped_devices() -> Result<Vec<DeviceWeightResponse>> {
    let mut devices_list = ConfigShapedDevices::load()?.devices;
    devices_list.sort_by(|a, b| a.circuit_id.cmp(&b.circuit_id));
    let mut result = vec![];
    let mut prev_id = String::new();
    for device in devices_list {
        let circuit_id = device.circuit_id;
        if circuit_id == prev_id {
            continue;
        }
        // Convert f32 to weight with proper rounding and minimum safeguards
        let weight = f32::max(1.0, device.download_max_mbps as f32 / 2.0).round() as i64;
        result.push(DeviceWeightResponse {
            circuit_id: circuit_id.clone(),
            weight,
        });
        prev_id = circuit_id.clone();
    }

    Ok(result)
}

/// This function is used to determine if we should use the Long Term Stats API to get the weights
fn use_lts_weights() -> bool {
    // Basic config gates first
    let Ok(config) = load_config() else {
        return false;
    };
    if !(config.long_term_stats.gather_stats && config.long_term_stats.license_key.is_some()) {
        return false;
    }
    // Ask lqosd (via bus) whether Insight is actually enabled/licensed
    if let Ok(responses) = crate::blocking::run_query(vec![BusRequest::CheckInsight]) {
        for resp in responses.into_iter() {
            if let BusResponse::InsightStatus(enabled) = resp {
                return enabled;
            }
        }
    }
    false
}

/// This function is used to get the device weights from the Long Term Stats API and the ShapedDevices.csv file
/// It will merge the two sources of weights and return the result as a Vec<DeviceWeightResponse>.
///
/// It serves as the Rust-side implementation of the `get_weights` function in the Python module.
pub(crate) fn get_weights_rust() -> Result<Vec<DeviceWeightResponse>> {
    let mut shaped_devices_weights = get_weights_from_shaped_devices()?;
    if use_lts_weights() {
        // This allows us to use Python printing
        println!("Using LTS weights");

        let config = load_config().unwrap();
        let org_key = config.long_term_stats.license_key.clone().unwrap();
        let node_id = config.node_id.clone();

        // Get current local time as unix timestamp
        let now = chrono::Utc::now().timestamp();
        // Subtract 7 days to get the start time
        let start = now - (60 * 60 * 24 * 7);

        let duration_seconds = 60 * 60 * 24; // 1 day
        let percentile = 0.95;

        eprintln!("Getting weights from LTS");
        let weights = get_weights_from_lts(&org_key, &node_id, start, duration_seconds, percentile);
        if let Ok(weights) = weights {
            eprintln!("Retrieved {} weights from LTS", weights.len());
            // Merge them
            for weight in weights.iter() {
                if let Some(existing) = shaped_devices_weights
                    .iter_mut()
                    .find(|d| d.circuit_id == weight.circuit_id)
                {
                    existing.weight = weight.weight;
                }
            }
        } else {
            eprintln!("Failed to get weights from LTS: {:?}", weights);
        }
    }

    Ok(shaped_devices_weights)
}

fn recurse_weights(
    device_list: &[ShapedDevice],
    device_weights: &[DeviceWeightResponse],
    network: &lqos_config::NetworkJson,
    node_index: usize,
) -> Result<i64> {
    let mut weight = 0;
    let n = &network.get_nodes_when_ready()[node_index];
    //println!("     Tower: {}", n.name);

    device_list
        .iter()
        .filter(|d| d.parent_node == n.name)
        .for_each(|d| {
            if let Some(w) = device_weights.iter().find(|w| w.circuit_id == d.circuit_id) {
                weight += w.weight;
            }
        });
    //println!("     Weight: {}", weight);

    for (i, _n) in network
        .get_nodes_when_ready()
        .iter()
        .enumerate()
        .filter(|(_i, n)| n.immediate_parent == Some(node_index))
    {
        //println!("     Child: {}", n.name);
        weight += recurse_weights(device_list, device_weights, network, i)?;
    }
    Ok(weight)
}

#[pyclass]
pub struct NetworkNodeWeight {
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub weight: i64,
}

/// Calculate the top-level network tree nodes and then
/// calculate the weights for each node
pub(crate) fn calculate_tree_weights() -> Result<Vec<NetworkNodeWeight>> {
    let device_list = ConfigShapedDevices::load()?.devices;
    let device_weights = get_weights_rust()?;
    let network = lqos_config::NetworkJson::load()?;
    let root_index = network
        .get_nodes_when_ready()
        .iter()
        .position(|n| n.immediate_parent.is_none())
        .unwrap();
    let mut result = Vec::new();
    //println!("Root index is: {}", root_index);

    // Find all network nodes one off the top
    network
        .get_nodes_when_ready()
        .iter()
        .enumerate()
        .filter(|(_, n)| n.immediate_parent.is_some() && n.immediate_parent.unwrap() == root_index)
        .for_each(|(idx, n)| {
            //println!("Node: {} ", n.name);
            let weight = recurse_weights(&device_list, &device_weights, &network, idx).unwrap();
            //println!("Node: {} : {weight}", n.name);
            result.push(NetworkNodeWeight {
                name: n.name.clone(),
                weight,
            });
        });

    result.sort_by(|a, b| b.weight.cmp(&a.weight));
    Ok(result)
}
