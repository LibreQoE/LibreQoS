use axum::Json;
use serde::Serialize;
use crate::node_manager::local_api::unknown_ips::get_unknown_ips;
use crate::shaped_devices_tracker::SHAPED_DEVICES;

#[derive(Serialize)]
pub struct DeviceCount {
    pub shaped_devices: usize,
    pub unknown_ips: usize,
}

pub async fn count_users() -> Json<DeviceCount> {
    Json(DeviceCount{
        shaped_devices: SHAPED_DEVICES.load().devices.len(),
        unknown_ips: get_unknown_ips().await.len(),
    })
}
