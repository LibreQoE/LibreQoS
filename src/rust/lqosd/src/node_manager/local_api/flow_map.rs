use axum::Json;
use crate::throughput_tracker::flow_data;

pub async fn flow_lat_lon() -> Json<Vec<(f64, f64, String, u64, f32)>> {
    Json(flow_data::RECENT_FLOWS.lat_lon_endpoints())
}