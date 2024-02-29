use lqos_bus::{bus_request, BusRequest, BusResponse, FlowbeeData};
use rocket::serde::json::Json;
use crate::cache_control::NoCache;

#[get("/api/flows/dump_all")]
pub async fn all_flows_debug_dump() -> NoCache<Json<Vec<FlowbeeData>>> {
  let responses =
    bus_request(vec![BusRequest::DumpActiveFlows]).await.unwrap();
  let result = match &responses[0] {
    BusResponse::AllActiveFlows(flowbee) => flowbee.to_owned(),
    _ => Vec::new(),
  };

  NoCache::new(Json(result))
}

#[get("/api/flows/count")]
pub async fn count_flows() -> NoCache<Json<u64>> {
  let responses =
    bus_request(vec![BusRequest::CountActiveFlows]).await.unwrap();
  let result = match &responses[0] {
    BusResponse::CountActiveFlows(count) => *count,
    _ => 0,
  };

  NoCache::new(Json(result))
}

#[get("/api/flows/top/<top_n>/<flow_type>")]
pub async fn top_5_flows(top_n: u32, flow_type: String) -> NoCache<Json<Vec<FlowbeeData>>> {
  let flow_type = match flow_type.as_str() {
    "rate" => lqos_bus::TopFlowType::RateEstimate,
    "bytes" => lqos_bus::TopFlowType::Bytes,
    "packets" => lqos_bus::TopFlowType::Packets,
    "drops" => lqos_bus::TopFlowType::Drops,
    "rtt" => lqos_bus::TopFlowType::RoundTripTime,
    _ => lqos_bus::TopFlowType::RateEstimate,
  };

  let responses =
    bus_request(vec![BusRequest::TopFlows { n: top_n, flow_type }]).await.unwrap();
  let result = match &responses[0] {
    BusResponse::TopFlows(flowbee) => flowbee.to_owned(),
    _ => Vec::new(),
  };

  NoCache::new(Json(result))
}