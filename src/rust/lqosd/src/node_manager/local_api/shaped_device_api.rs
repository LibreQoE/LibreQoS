use axum::Json;
use lqos_config::ShapedDevice;
use crate::shaped_devices_tracker::SHAPED_DEVICES;

pub async fn all_shaped_devices() -> Json<Vec<ShapedDevice>> {
    Json(SHAPED_DEVICES.read().unwrap().devices.clone())
}