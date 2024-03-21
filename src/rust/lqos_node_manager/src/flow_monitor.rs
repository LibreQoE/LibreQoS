use lqos_bus::{bus_request, BusRequest, BusResponse, FlowbeeSummaryData};
use rocket::serde::json::Json;
use crate::cache_control::NoCache;

#[get("/api/flows/dump_all")]
pub async fn all_flows_debug_dump() -> NoCache<Json<Vec<FlowbeeSummaryData>>> {
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
pub async fn top_5_flows(top_n: u32, flow_type: String) -> NoCache<Json<Vec<FlowbeeSummaryData>>> {
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

#[get("/api/flows/by_country")]
pub async fn flows_by_country() -> NoCache<Json<Vec<(String, [u64; 2], [f32; 2])>>> {
  let responses =
    bus_request(vec![BusRequest::CurrentEndpointsByCountry]).await.unwrap();
  let result = match &responses[0] {
    BusResponse::CurrentEndpointsByCountry(country_summary) => country_summary.to_owned(),
    _ => Vec::new(),
  };

  NoCache::new(Json(result))
}

#[get("/api/flows/lat_lon")]
pub async fn flows_lat_lon() -> NoCache<Json<Vec<(f64, f64, String, u64, f32)>>> {
  let responses =
    bus_request(vec![BusRequest::CurrentEndpointLatLon]).await.unwrap();
  let result = match &responses[0] {
    BusResponse::CurrentLatLon(lat_lon) => lat_lon.to_owned(),
    _ => Vec::new(),
  };

  NoCache::new(Json(result))
}

#[get("/api/flows/ether_protocol")]
pub async fn flows_ether_protocol() -> NoCache<Json<BusResponse>> {
  let responses =
    bus_request(vec![BusRequest::EtherProtocolSummary]).await.unwrap();
  let result = responses[0].to_owned();

  NoCache::new(Json(result))
}

#[get("/api/flows/ip_protocol")]
pub async fn flows_ip_protocol() -> NoCache<Json<Vec<(String, (u64, u64))>>> {
  let responses =
    bus_request(vec![BusRequest::IpProtocolSummary]).await.unwrap();
  let result = match &responses[0] {
    BusResponse::IpProtocols(ip_protocols) => ip_protocols.to_owned(),
    _ => Vec::new(),
  };

  NoCache::new(Json(result))
}