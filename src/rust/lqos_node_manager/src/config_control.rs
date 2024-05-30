use crate::{auth_guard::AuthGuard, cache_control::NoCache};
use default_net::get_interfaces;
use lqos_bus::{bus_request, BusRequest, BusResponse};
use lqos_config::{Tunables, Config, ShapedDevice};
use rocket::{fs::NamedFile, serde::{json::Json, Serialize, Deserialize}};
use rocket::serde::json::Value;
use crate::tracker::SHAPED_DEVICES;

#[get("/api/node_name")]
pub async fn get_node_name() -> Json<String> {
  if let Ok(config) = lqos_config::load_config() {
    Json(config.node_name)
  } else {
    Json("No Name Provided".to_string())  
  }
}

// Note that NoCache can be replaced with a cache option
// once the design work is complete.
#[get("/config")]
pub async fn config_page<'a>(_auth: AuthGuard) -> NoCache<Option<NamedFile>> {
  NoCache::new(NamedFile::open("static/config.html").await.ok())
}

#[get("/api/list_nics")]
pub async fn get_nic_list<'a>(
  _auth: AuthGuard,
) -> NoCache<Json<Vec<(String, String, String)>>> {
  let result = get_interfaces()
    .iter()
    .map(|eth| {
      let mac = if let Some(mac) = &eth.mac_addr {
        mac.to_string()
      } else {
        String::new()
      };
      (eth.name.clone(), format!("{:?}", eth.if_type), mac)
    })
    .collect();

  NoCache::new(Json(result))
}

#[get("/api/config")]
pub async fn get_current_lqosd_config(
  _auth: AuthGuard,
) -> NoCache<Json<Config>> {
  let config = lqos_config::load_config().unwrap();
  println!("{config:#?}");
  NoCache::new(Json(config))
}

#[post("/api/update_config", data = "<data>")]
pub async fn update_lqosd_config(
  data: Json<Config>
) -> String {
  let config: Config = (*data).clone();
  bus_request(vec![BusRequest::UpdateLqosdConfig(Box::new(config))])
      .await
      .unwrap();
  "Ok".to_string()
}

#[derive(Deserialize, Clone)]
#[serde(crate = "rocket::serde")]
pub struct NetworkAndDevices {
  shaped_devices: Vec<ShapedDevice>,
  network_json: Value,
}

#[post("/api/update_network_and_devices", data = "<data>")]
pub async fn update_network_and_devices(
  data: Json<NetworkAndDevices>
) -> String {
  let config = lqos_config::load_config().unwrap();

  // Save network.json
  let serialized_string = rocket::serde::json::to_pretty_string(&data.network_json).unwrap();
  let net_json_path = std::path::Path::new(&config.lqos_directory).join("network.json");
  let net_json_backup_path = std::path::Path::new(&config.lqos_directory).join("network.json.backup");
  if net_json_path.exists() {
    // Make a backup
    std::fs::copy(&net_json_path, net_json_backup_path).unwrap();
  }
  std::fs::write(net_json_path, serialized_string).unwrap();

  // Save the Shaped Devices
  let sd_path = std::path::Path::new(&config.lqos_directory).join("ShapedDevices.csv");
  let sd_backup_path = std::path::Path::new(&config.lqos_directory).join("ShapedDevices.csv.backup");
  if sd_path.exists() {
    std::fs::copy(&sd_path, sd_backup_path).unwrap();
  }
  let mut lock = SHAPED_DEVICES.write().unwrap();
  lock.replace_with_new_data(data.shaped_devices.clone());
  println!("{:?}", lock.devices);
  lock.write_csv(&format!("{}/ShapedDevices.csv", config.lqos_directory)).unwrap();

  "Ok".to_string()
}

#[post("/api/lqos_tuning/<period>", data = "<tuning>")]
pub async fn update_lqos_tuning(
  auth: AuthGuard,
  period: u64,
  tuning: Json<Tunables>,
) -> Json<String> {
  if auth != AuthGuard::Admin {
    return Json("Error: Not authorized".to_string());
  }

  // Send the update to the server
  bus_request(vec![BusRequest::UpdateLqosDTuning(period, (*tuning).clone())])
    .await
    .unwrap();

  // For now, ignore the reply.

  Json("OK".to_string())
}

#[derive(Serialize, Clone, Default)]
#[serde(crate = "rocket::serde")]
pub struct LqosStats {
  pub bus_requests_since_start: u64,
  pub time_to_poll_hosts_us: u64,
  pub high_watermark: (u64, u64),
  pub tracked_flows: u64,
  pub rtt_events_per_second: u64,
}

#[get("/api/stats")]
pub async fn stats() -> NoCache<Json<LqosStats>> {
  for msg in bus_request(vec![BusRequest::GetLqosStats]).await.unwrap() {
    if let BusResponse::LqosdStats { bus_requests, time_to_poll_hosts, high_watermark, tracked_flows, rtt_events_per_second } = msg {
      return NoCache::new(Json(LqosStats {
        bus_requests_since_start: bus_requests,
        time_to_poll_hosts_us: time_to_poll_hosts,
        high_watermark,
        tracked_flows,
        rtt_events_per_second,
      }));
    }
  }
  NoCache::new(Json(LqosStats::default()))
}