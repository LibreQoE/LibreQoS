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
use lqos_config::{load_config, ConfigShapedDevices};
use pyo3::pyclass;
use serde::{Deserialize, Serialize};

const URL: &str = "http:/localhost:9127/api/device_weights";

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

    // Make a BLOCKING reqwest call (we're not in an async context)
    let response = reqwest::blocking::Client::new()
        .post(URL)
        .json(&request)
        .send()?
        .json::<Vec<DeviceWeightResponse>>()?;

    Ok(response)
}

/// This function is used to get the device weights from the ShapedDevices.csv file
fn get_weights_from_shaped_devices() -> Result<Vec<DeviceWeightResponse>> {
    let mut devices_list = ConfigShapedDevices::load()?.devices;
    devices_list.sort_by(|a,b| a.circuit_id.cmp(&b.circuit_id));
    let mut result = vec![];
    let mut prev_id = String::new();
    for device in devices_list {
        let circuit_id = device.circuit_id;
        if circuit_id == prev_id {
            continue;
        }
        let weight = device.download_max_mbps as i64 / 2;
        result.push(DeviceWeightResponse { circuit_id: circuit_id.clone(), weight });
        prev_id = circuit_id.clone();
    }

    Ok(result)
}

/// This function is used to determine if we should use the Long Term Stats API to get the weights
fn use_lts_weights() -> bool {
    let config = load_config().unwrap();
    config.long_term_stats.gather_stats && config.long_term_stats.license_key.is_some()
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
        let org_key = config.long_term_stats.license_key.unwrap();
        let node_id = config.node_id;

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
                if let Some(existing) = shaped_devices_weights.iter_mut().find(|d| d.circuit_id == weight.circuit_id) {                    
                    existing.weight = weight.weight;
                }
            }
        } else {
            eprintln!("Failed to get weights from LTS: {:?}", weights);
        }
    } else {
        eprintln!("Not using LTS weights. Using weights from ShapedDevices.csv");
    }

    Ok(shaped_devices_weights)
}
