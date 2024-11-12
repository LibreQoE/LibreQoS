use axum::Json;
use lqos_bus::{bus_request, BusRequest, BusResponse};

pub async fn flow_lat_lon() -> Json<Vec<(f64, f64, String, u64, f32)>> {
    let Ok(replies) = bus_request(vec![BusRequest::FlowLatLon]).await else {
        return Json(Vec::new());
    };
    for reply in replies.into_iter() {
        if let BusResponse::FlowLatLon(data) = reply {
            return Json(data);
        }
    }

    Json(Vec::new())
}