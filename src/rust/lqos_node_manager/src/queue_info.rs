use crate::auth_guard::AuthGuard;
use crate::cache_control::NoCache;
use crate::tracker::SHAPED_DEVICES;
use lqos_bus::{
    decode_response, encode_request, BusRequest, BusResponse, BusSession, BUS_BIND_ADDRESS,
};
use rocket::response::content::RawJson;
use rocket::serde::json::Json;
use rocket::serde::Serialize;
use rocket::tokio::io::{AsyncReadExt, AsyncWriteExt};
use rocket::tokio::net::TcpStream;
use std::net::IpAddr;

#[derive(Serialize, Clone)]
#[serde(crate = "rocket::serde")]
pub struct CircuitInfo {
    pub name: String,
    pub capacity: (u64, u64),
}

#[get("/api/watch_circuit/<circuit_id>")]
pub async fn watch_circuit(circuit_id: String, _auth: AuthGuard) -> NoCache<Json<String>> {
    let mut stream = TcpStream::connect(BUS_BIND_ADDRESS).await.unwrap();
    let test = BusSession {
        auth_cookie: 1234,
        requests: vec![BusRequest::WatchQueue(circuit_id)],
    };
    let msg = encode_request(&test).unwrap();
    stream.write(&msg).await.unwrap();

    // Receive reply
    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf).await.unwrap();
    let _reply = decode_response(&buf).unwrap();

    NoCache::new(Json("OK".to_string()))
}

#[get("/api/circuit_info/<circuit_id>")]
pub async fn circuit_info(circuit_id: String, _auth: AuthGuard) -> NoCache<Json<CircuitInfo>> {
    if let Some(device) = SHAPED_DEVICES
        .read()
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
        NoCache::new(Json(result))
    } else {
        let result = CircuitInfo {
            name: "Nameless".to_string(),
            capacity: (1_000_000, 1_000_000),
        };
        NoCache::new(Json(result))
    }
}

#[get("/api/circuit_throughput/<circuit_id>")]
pub async fn current_circuit_throughput(
    circuit_id: String,
    _auth: AuthGuard,
) -> NoCache<Json<Vec<(String, u64, u64)>>> {
    let mut result = Vec::new();
    // Get a list of host counts
    // This is really inefficient, but I'm struggling to find a better way.
    // TODO: Fix me up
    let mut stream = TcpStream::connect(BUS_BIND_ADDRESS).await.unwrap();
    let test = BusSession {
        auth_cookie: 1234,
        requests: vec![BusRequest::GetHostCounter],
    };
    let msg = encode_request(&test).unwrap();
    stream.write(&msg).await.unwrap();

    // Receive reply
    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf).await.unwrap();
    let reply = decode_response(&buf).unwrap();
    for msg in reply.responses.iter() {
        match msg {
            BusResponse::HostCounters(hosts) => {
                let devices = SHAPED_DEVICES.read();
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
            _ => {}
        }
    }

    NoCache::new(Json(result))
}

#[get("/api/raw_queue_by_circuit/<circuit_id>")]
pub async fn raw_queue_by_circuit(
    circuit_id: String,
    _auth: AuthGuard,
) -> NoCache<RawJson<String>> {
    let mut stream = TcpStream::connect(BUS_BIND_ADDRESS).await.unwrap();
    let test = BusSession {
        auth_cookie: 1234,
        requests: vec![BusRequest::GetRawQueueData(circuit_id)],
    };
    let msg = encode_request(&test).unwrap();
    stream.write(&msg).await.unwrap();

    // Receive reply
    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf).await.unwrap();
    let reply = decode_response(&buf).unwrap();

    let result = match &reply.responses[0] {
        BusResponse::RawQueueData(msg) => msg.clone(),
        _ => "Unable to request queue".to_string(),
    };
    NoCache::new(RawJson(result))
}

#[cfg(feature = "equinix_tests")]
#[get("/api/run_btest")]
pub async fn run_btest() -> NoCache<RawJson<String>> {
    let mut stream = TcpStream::connect(BUS_BIND_ADDRESS).await.unwrap();
    let test = BusSession {
        auth_cookie: 1234,
        requests: vec![BusRequest::RequestLqosEquinixTest],
    };
    let msg = encode_request(&test).unwrap();
    stream.write(&msg).await.unwrap();

    // Receive reply
    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf).await.unwrap();
    let reply = decode_response(&buf).unwrap();

    let result = match &reply.responses[0] {
        BusResponse::Ack => String::new(),
        _ => "Unable to request test".to_string(),
    };
    NoCache::new(RawJson(result))
}

#[cfg(not(feature = "equinix_tests"))]
pub async fn run_btest() -> NoCache<RawJson<String>> {
    NoCache::new(RawJson("No!"))
}
