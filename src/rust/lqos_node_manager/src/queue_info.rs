use crate::auth_guard::AuthGuard;
use crate::cache_control::NoCache;
use crate::tracker::{SHAPED_DEVICES, lookup_dns};
use lqos_bus::{bus_request, BusRequest, BusResponse, FlowTransport, PacketHeader, QueueStoreTransit};
use rocket::fs::NamedFile;
use rocket::http::Status;
use rocket::response::content::RawJson;
use rocket::serde::json::Json;
use rocket::serde::Serialize;
use rocket::serde::msgpack::MsgPack;
use std::net::IpAddr;

#[derive(Serialize, Clone)]
#[serde(crate = "rocket::serde")]
pub struct CircuitInfo {
  pub name: String,
  pub capacity: (u64, u64),
}

#[get("/api/watch_circuit/<circuit_id>")]
pub async fn watch_circuit(
  circuit_id: String,
  _auth: AuthGuard,
) -> NoCache<Json<String>> {
  bus_request(vec![BusRequest::WatchQueue(circuit_id)]).await.unwrap();

  NoCache::new(Json("OK".to_string()))
}

#[get("/api/circuit_info/<circuit_id>")]
pub async fn circuit_info(
  circuit_id: String,
  _auth: AuthGuard,
) -> NoCache<MsgPack<CircuitInfo>> {
  if let Some(device) = SHAPED_DEVICES
    .read()
    .unwrap()
    .devices
    .iter()
    .find(|d| d.circuit_id == circuit_id)
  {
    let result = CircuitInfo {
      name: device.circuit_name.clone(),
      capacity: (
        device.download_max_mbps as u64 * 1_000_000,
        device.upload_max_mbps as u64 * 1_000_000,
      ),
    };
    NoCache::new(MsgPack(result))
  } else {
    let result = CircuitInfo {
      name: "Nameless".to_string(),
      capacity: (1_000_000, 1_000_000),
    };
    NoCache::new(MsgPack(result))
  }
}

#[get("/api/circuit_throughput/<circuit_id>")]
pub async fn current_circuit_throughput(
  circuit_id: String,
  _auth: AuthGuard,
) -> NoCache<MsgPack<Vec<(String, u64, u64)>>> {
  let mut result = Vec::new();
  // Get a list of host counts
  // This is really inefficient, but I'm struggling to find a better way.
  // TODO: Fix me up

  for msg in
    bus_request(vec![BusRequest::GetHostCounter]).await.unwrap().iter()
  {
    if let BusResponse::HostCounters(hosts) = msg {
      let devices = SHAPED_DEVICES.read().unwrap();
      for (ip, down, up) in hosts.iter() {
        let lookup = match ip {
          IpAddr::V4(ip) => ip.to_ipv6_mapped(),
          IpAddr::V6(ip) => *ip,
        };
        if let Some(c) = devices.trie.longest_match(lookup) {
          if devices.devices[*c.1].circuit_id == circuit_id {
            result.push((ip.to_string(), *down, *up));
          }
        }
      }
    }
  }

  NoCache::new(MsgPack(result))
}

#[get("/api/raw_queue_by_circuit/<circuit_id>")]
pub async fn raw_queue_by_circuit(
  circuit_id: String,
  _auth: AuthGuard,
) -> NoCache<MsgPack<QueueStoreTransit>> {

  let responses =
    bus_request(vec![BusRequest::GetRawQueueData(circuit_id)]).await.unwrap();

  let result = match &responses[0] {
    BusResponse::RawQueueData(Some(msg)) => {
      *msg.clone()
    }
    _ => QueueStoreTransit::default()
  };
  NoCache::new(MsgPack(result))
}

#[get("/api/flows/<ip_list>")]
pub async fn flow_stats(ip_list: String, _auth: AuthGuard) -> NoCache<MsgPack<Vec<(FlowTransport, Option<FlowTransport>)>>> {
  let mut result = Vec::new();
  let request: Vec<BusRequest> = ip_list.split(',').map(|ip| BusRequest::GetFlowStats(ip.to_string())).collect();
  let responses = bus_request(request).await.unwrap();
  for r in responses.iter() {
    if let BusResponse::FlowData(flow) = r {
      result.extend_from_slice(flow);
    }
  }
  NoCache::new(MsgPack(result))
}

#[derive(Serialize, Clone)]
#[serde(crate = "rocket::serde")]
pub enum RequestAnalysisResult {
  Fail,
  Ok{ session_id: usize, countdown: usize }
}

#[get("/api/request_analysis/<ip>")]
pub async fn request_analysis(ip: String) -> NoCache<Json<RequestAnalysisResult>> {
  for r in bus_request(vec![BusRequest::GatherPacketData(ip)]).await.unwrap() {
    if let BusResponse::PacketCollectionSession{session_id, countdown} = r {
      return NoCache::new(Json(RequestAnalysisResult::Ok{session_id, countdown}));
    }
  }

  NoCache::new(Json(RequestAnalysisResult::Fail))
}

#[get("/api/packet_dump/<id>")]
pub async fn packet_dump(id: usize, _auth: AuthGuard) -> NoCache<Json<Vec<PacketHeader>>> {
  let mut result = Vec::new();
  for r in bus_request(vec![BusRequest::GetPacketHeaderDump(id)]).await.unwrap() {
    if let BusResponse::PacketDump(Some(packets)) = r {
      result.extend(packets);
    }
  }
  NoCache::new(Json(result))
}

#[allow(unused_variables)]
#[get("/api/pcap/<id>/<filename>")]
pub async fn pcap(id: usize, filename: String) -> Result<NoCache<NamedFile>, Status> {
  // The unusued _filename parameter is there to allow the changing of the
  // filename on the client side. See Github issue 291.
  for r in bus_request(vec![BusRequest::GetPcapDump(id)]).await.unwrap() {
    if let BusResponse::PcapDump(Some(filename)) = r {
      return Ok(NoCache::new(NamedFile::open(filename).await.unwrap()));
    }
  }

  Err(Status::NotFound)
}

#[get("/api/dns/<ip>")]
pub async fn dns_query(ip: String) -> NoCache<String> {
  if let Ok(ip) = ip.parse::<IpAddr>() {
    NoCache::new(lookup_dns(ip))
  } else {
    NoCache::new(ip)
  }
}

#[cfg(feature = "equinix_tests")]
#[get("/api/run_btest")]
pub async fn run_btest() -> NoCache<RawJson<String>> {
  let responses =
    bus_request(vec![BusRequest::RequestLqosEquinixTest]).await.unwrap();
  let result = match &responses[0] {
    BusResponse::Ack => String::new(),
    _ => "Unable to request test".to_string(),
  };
  NoCache::new(RawJson(result))
}

#[cfg(not(feature = "equinix_tests"))]
pub async fn run_btest() -> NoCache<RawJson<String>> {
  NoCache::new(RawJson("No!"))
}
