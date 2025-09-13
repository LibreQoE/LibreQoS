//! LTS1 (legacy) statistics have been removed. These stubs keep
//! bus handlers compiling, returning a clear "No Data" response.
use lqos_bus::BusResponse;

pub fn get_stats_totals() -> BusResponse {
    BusResponse::Fail("No Data".to_string())
}

pub fn get_stats_host() -> BusResponse {
    BusResponse::Fail("No Data".to_string())
}

pub fn get_stats_tree() -> BusResponse {
    BusResponse::Fail("No Data".to_string())
}
