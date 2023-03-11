use std::{time::Duration, net::IpAddr};

use dashmap::DashMap;
use lqos_bus::{BusResponse, FlowTransport};
use lqos_sys::{PalantirData, PalantirKey, XdpIpAddress};
use lqos_utils::unix_time::time_since_boot;
use once_cell::sync::Lazy;

use crate::stats::FLOWS_TRACKED;

pub(crate) static PALANTIR: Lazy<PalantirMonitor> =
  Lazy::new(PalantirMonitor::new);

pub(crate) struct PalantirMonitor {
  pub(crate) data: DashMap<PalantirKey, FlowData>,
}

#[derive(Default)]
pub(crate) struct FlowData {
  last_seen: u64,
  bytes: u64,
  packets: u64,
  tos: u8,
}

impl PalantirMonitor {
  fn new() -> Self {
    Self { data: DashMap::new() }
  }

  fn combine_flows(values: &[PalantirData]) -> FlowData {
    let mut result = FlowData::default();
    let mut ls = 0;
    values.iter().for_each(|v| {
      result.bytes += v.bytes;
      result.packets += v.packets;
      result.tos += v.tos;
      if v.last_seen > ls {
        ls = v.last_seen;
      }
    });
    result.last_seen = ls;
    result
  }

  pub(crate) fn ingest(&self, key: &PalantirKey, values: &[PalantirData]) {
    //println!("{key:?}");
    //println!("{values:?}");
    if let Some(expire_ns) = Self::get_expire_time() {
      let combined = Self::combine_flows(values);
      if combined.last_seen > expire_ns {
        if let Some(mut flow) = self.data.get_mut(key) {
          // Update
          flow.bytes = combined.bytes;
          flow.packets = combined.packets;
          flow.last_seen = combined.last_seen;
          if combined.tos != 0 {
            flow.tos = combined.tos;
          }
        } else {
          // Insert
          self.data.insert(key.clone(), combined);
        }
      }
    }
  }

  fn get_expire_time() -> Option<u64> {
    let boot_time = time_since_boot();
    if let Ok(boot_time) = boot_time {
      let time_since_boot = Duration::from(boot_time);
      let five_minutes_ago =
        time_since_boot.saturating_sub(Duration::from_secs(30));
      let expire_ns = five_minutes_ago.as_nanos() as u64;
      Some(expire_ns)
    } else {
      None
    }
  }

  pub(crate) fn expire(&self) {
    if let Some(expire_ns) = Self::get_expire_time() {
      self.data.retain(|_k, v| v.last_seen > expire_ns);
    }
    FLOWS_TRACKED.store(self.data.len() as u64, std::sync::atomic::Ordering::Relaxed);
  }
}

pub fn get_flow_stats(ip: &str) -> BusResponse {
  let ip = ip.parse::<IpAddr>();
  if let Ok(ip) = ip {
    let ip = XdpIpAddress::from_ip(ip);
    let mut result = Vec::new();

    for value in PALANTIR.data.iter() {
      let key = value.key();
      if key.src_ip == ip || key.dst_ip == ip {
        result.push(FlowTransport{
          src: key.src_ip.as_ip().to_string(),
          dst: key.dst_ip.as_ip().to_string(),
          src_port: key.src_port,
          dst_port: key.dst_port,
          proto: match key.ip_protocol {
            6 => lqos_bus::FlowProto::TCP,
            17 => lqos_bus::FlowProto::UDP,
            _ => lqos_bus::FlowProto::ICMP,
          },
          bytes: value.bytes,
          packets: value.packets,
          tos: value.tos,
        });
      }
    }

    return BusResponse::FlowData(result);
  }
  BusResponse::Fail("No Stats or bad IP".to_string())
}