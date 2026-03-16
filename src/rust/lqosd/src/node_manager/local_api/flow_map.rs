use crate::throughput_tracker::flow_data;

pub fn flow_map_data() -> Vec<(f64, f64, String, u64, f32)> {
    flow_data::RECENT_FLOWS.lat_lon_endpoints()
}

pub fn endpoints_by_country_data() -> Vec<(
    String,
    lqos_utils::units::DownUpOrder<u64>,
    [f32; 2],
    String,
)> {
    flow_data::RECENT_FLOWS.country_summary()
}

pub fn endpoint_latlon_data() -> Vec<(f64, f64, String, u64, f32)> {
    flow_data::RECENT_FLOWS.lat_lon_endpoints()
}
