use std::net::IpAddr;
use lqos_bus::BusResponse;
use lqos_sys::heimdall_watch_ip;
use lqos_utils::XdpIpAddress;

pub fn get_flow_stats(ip: &str) -> BusResponse {
  let ip = ip.parse::<IpAddr>();
  if let Ok(ip) = ip {
    let ip = XdpIpAddress::from_ip(ip);
    heimdall_watch_ip(ip);
    return lqos_heimdall::get_flow_stats(ip);
  }
  BusResponse::Fail("No Stats or bad IP".to_string())
}