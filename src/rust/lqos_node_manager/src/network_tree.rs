use std::net::IpAddr;

use lqos_bus::{bus_request, BusRequest, BusResponse};
use lqos_config::NetworkJsonTransport;
use rocket::{
  fs::NamedFile,
  serde::{json::Json, Serialize, msgpack::MsgPack},
};
use rocket::serde::json::Value;

use crate::{cache_control::NoCache, tracker::SHAPED_DEVICES};

// Note that NoCache can be replaced with a cache option
// once the design work is complete.
#[get("/tree")]
pub async fn tree_page<'a>() -> NoCache<Option<NamedFile>> {
  NoCache::new(NamedFile::open("static/tree.html").await.ok())
}

#[get("/api/network_tree/<parent>")]
pub async fn tree_entry(
  parent: usize,
) -> NoCache<MsgPack<Vec<(usize, NetworkJsonTransport)>>> {
  let responses =
    bus_request(vec![BusRequest::GetNetworkMap { parent }]).await.unwrap();
  let result = match &responses[0] {
    BusResponse::NetworkMap(nodes) => nodes.to_owned(),
    _ => Vec::new(),
  };

  NoCache::new(MsgPack(result))
}

#[get("/api/network_tree_summary")]
pub async fn network_tree_summary(
) -> NoCache<MsgPack<Vec<(usize, NetworkJsonTransport)>>> {
  let responses =
    bus_request(vec![BusRequest::TopMapQueues(4)]).await.unwrap();
  let result = match &responses[0] {
    BusResponse::NetworkMap(nodes) => nodes.to_owned(),
    _ => Vec::new(),
  };
  NoCache::new(MsgPack(result))
}

#[derive(Serialize, Clone)]
#[serde(crate = "rocket::serde")]
pub struct CircuitThroughput {
  pub id: String,
  pub name: String,
  pub traffic: (u64, u64),
  pub limit: (u64, u64),
}

#[get("/api/tree_clients/<parent>")]
pub async fn tree_clients(
  parent: String,
) -> NoCache<MsgPack<Vec<CircuitThroughput>>> {
  let mut result = Vec::new();
  for msg in
    bus_request(vec![BusRequest::GetHostCounter]).await.unwrap().iter()
  {
    let devices = SHAPED_DEVICES.read().unwrap();
    if let BusResponse::HostCounters(hosts) = msg {
      for (ip, down, up) in hosts.iter() {
        let lookup = match ip {
          IpAddr::V4(ip) => ip.to_ipv6_mapped(),
          IpAddr::V6(ip) => *ip,
        };
        if let Some(c) = devices.trie.longest_match(lookup) {
          if devices.devices[*c.1].parent_node == parent {
            result.push(CircuitThroughput {
              id: devices.devices[*c.1].circuit_id.clone(),
              name: devices.devices[*c.1].circuit_name.clone(),
              traffic: (*down, *up),
              limit: (
                devices.devices[*c.1].download_max_mbps as u64,
                devices.devices[*c.1].upload_max_mbps as u64,
              ),
            });
          }
        }
      }
    }
  }
  NoCache::new(MsgPack(result))
}

#[post("/api/node_names", data = "<nodes>")]
pub async fn node_names(
  nodes: Json<Vec<usize>>,
) -> NoCache<Json<Vec<(usize, String)>>> {
  let mut result = Vec::new();
  for msg in bus_request(vec![BusRequest::GetNodeNamesFromIds(nodes.0)])
    .await
    .unwrap()
    .iter()
  {
    if let BusResponse::NodeNames(map) = msg {
      result.extend_from_slice(map);
    }
  }

  NoCache::new(Json(result))
}

#[get("/api/funnel_for_queue/<circuit_id>")]
pub async fn funnel_for_queue(
  circuit_id: String,
) -> NoCache<MsgPack<Vec<(usize, NetworkJsonTransport)>>> {
  let mut result = Vec::new();

  let target = SHAPED_DEVICES
    .read()
    .unwrap()
    .devices
    .iter()
    .find(|d| d.circuit_id == circuit_id)
    .as_ref()
    .unwrap()
    .parent_node
    .clone();

  for msg in
    bus_request(vec![BusRequest::GetFunnel { target }]).await.unwrap().iter()
  {
    if let BusResponse::NetworkMap(map) = msg {
      result.extend_from_slice(map);
    }
  }
  NoCache::new(MsgPack(result))
}

#[get("/api/network_json")]
pub async fn get_network_json() -> NoCache<Json<Value>> {
  if let Ok(config) = lqos_config::load_config() {
    let path = std::path::Path::new(&config.lqos_directory).join("network.json");
    if path.exists() {
      let raw = std::fs::read_to_string(path).unwrap();
      let json: Value = rocket::serde::json::from_str(&raw).unwrap();
      return NoCache::new(Json(json));
    }
  }

  NoCache::new(Json(Value::String("Not done yet".to_string())))
}