use lqos_bus::BusResponse;

pub fn get_flow_stats(_ip: &str) -> BusResponse {
  BusResponse::Fail("No Stats or bad IP".to_string())
}