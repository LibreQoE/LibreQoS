use axum::Json;
use lqos_bus::{bus_request, BusRequest, BusResponse};
use lqos_config::NetworkJsonTransport;
use crate::shaped_devices_tracker::NETWORK_JSON;

pub async fn get_network_tree() -> Json<Vec<(usize, NetworkJsonTransport)>> {
    let Ok(replies) = bus_request(vec![BusRequest::GetFullNetworkMap]).await else {
        return Json(vec![]);
    };
    for reply in replies {
        match reply {
            BusResponse::NetworkMap(map) => {
                return Json(map);
            }
            _ => {}
        }
    }
    Json(vec![])
}