use crate::throughput_tracker::flow_data;
pub fn flow_map_data() -> Vec<(f64, f64, String, u64, f32)> {
    flow_data::RECENT_FLOWS.lat_lon_endpoints()
}
