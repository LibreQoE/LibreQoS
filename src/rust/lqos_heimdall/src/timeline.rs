use std::{sync::RwLock, time::Duration};

use lqos_bus::PacketHeader;
use lqos_utils::{unix_time::time_since_boot, XdpIpAddress};
use once_cell::sync::Lazy;
use crate::perf_interface::HeimdallEvent;

impl From<&HeimdallEvent> for PacketHeader {
    fn from(value: &HeimdallEvent) -> Self {
        Self {
            timestamp: value.timestamp,
            src: value.src.as_ip().to_string(),
            dst: value.dst.as_ip().to_string(),
            src_port: value.src_port,
            dst_port: value.dst_port,
            ip_protocol: value.ip_protocol,
            tos: value.tos,
            size: value.size,
        }
    }
}

struct Timeline {
  data: RwLock<Vec<HeimdallEvent>>,
}

impl Timeline {
  fn new() -> Self {
    Self { data: RwLock::new(Vec::new()) }
  }
}

static TIMELINE: Lazy<Timeline> = Lazy::new(Timeline::new);

pub(crate) fn store_on_timeline(event: HeimdallEvent) {
  let mut lock = TIMELINE.data.write().unwrap();
  lock.push(event); // We're moving here deliberately
}

pub(crate) fn expire_timeline() {
  if let Ok(now) = time_since_boot() {
    let since_boot = Duration::from(now);
    let ten_secs_ago = since_boot - Duration::from_secs(10);
    let expire = ten_secs_ago.as_nanos() as u64;
    let mut lock = TIMELINE.data.write().unwrap();
    lock.retain(|v| v.timestamp > expire);
  }
}

pub fn ten_second_packet_dump(ip: XdpIpAddress) -> Vec<PacketHeader> {
  TIMELINE
    .data
    .read()
    .unwrap()
    .iter()
    .filter(|e| e.src == ip || e.dst == ip)
    .map(|e| e.into())
    .collect()
}
