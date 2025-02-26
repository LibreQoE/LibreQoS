use crate::shaped_devices_tracker::SHAPED_DEVICES;
use axum::Json;
use lqos_config::ShapedDevice;

pub async fn all_shaped_devices() -> Json<Vec<ShapedDevice>> {
    Json(SHAPED_DEVICES.load().devices.clone())
}
