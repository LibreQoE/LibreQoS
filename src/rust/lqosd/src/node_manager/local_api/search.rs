use axum::Json;
use serde::{Deserialize, Serialize};
use crate::shaped_devices_tracker::{NETWORK_JSON, SHAPED_DEVICES};

#[derive(Serialize, Deserialize, Debug)]
pub struct SearchRequest {
    pub term: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum SearchResult {
    Circuit { id: String, name: String },
    Device { circuit_id: String, name: String, circuit_name: String },
    Site { idx: usize, name: String },
}

pub async fn search(Json(search) : Json<SearchRequest>) -> Json<Vec<SearchResult>> {
    let mut results = vec![];
    let term = search.term.to_lowercase().trim().to_string();
    {
        let sd_reader = SHAPED_DEVICES.read().unwrap();
        sd_reader.devices.iter().for_each(|sd| {
            if sd.circuit_name.to_lowercase().trim().contains(&term) {
                results.push(SearchResult::Circuit { id: sd.circuit_id.clone(), name: sd.circuit_name.clone() });
            }
            if sd.device_name.to_lowercase().trim().contains(&term) {
                results.push(SearchResult::Device { circuit_id: sd.circuit_id.clone(), name: sd.device_name.clone(), circuit_name: sd.circuit_name.clone() });
            }
        });
    }
    {
        let net_reader = NETWORK_JSON.load();
        net_reader.get_nodes_when_ready().iter().enumerate().for_each(|(idx,n)| {
            if n.name.to_lowercase().trim().contains(&term) {
                match n.node_type.as_ref() {
                    _ => results.push(SearchResult::Site { idx, name: n.name.clone() }),
                }
            }
        });
    }
    Json(results)
}