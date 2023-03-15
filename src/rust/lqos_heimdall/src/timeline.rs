use std::time::Duration;
use dashmap::DashSet;
use lqos_bus::PacketHeader;
use lqos_utils::{unix_time::time_since_boot, XdpIpAddress};
use once_cell::sync::Lazy;
use crate::perf_interface::HeimdallEvent;

impl HeimdallEvent {
    fn as_header(&self) -> PacketHeader {
      PacketHeader {
            timestamp: self.timestamp,
            src: self.src.as_ip().to_string(),
            dst: self.dst.as_ip().to_string(),
            src_port: self.src_port,
            dst_port: self.dst_port,
            ip_protocol: self.ip_protocol,
            tos: self.tos,
            size: self.size,
        }
    }
}

struct Timeline {
  data: DashSet<HeimdallEvent>,
}

impl Timeline {
  fn new() -> Self {
    Self { data: DashSet::new() }
  }
}

static TIMELINE: Lazy<Timeline> = Lazy::new(Timeline::new);

pub(crate) fn store_on_timeline(event: HeimdallEvent) {
  TIMELINE.data.insert(event); // We're moving here deliberately
}

pub(crate) fn expire_timeline() {
  if let Ok(now) = time_since_boot() {
    let since_boot = Duration::from(now);
    let ten_secs_ago = since_boot - Duration::from_secs(10);
    let expire = ten_secs_ago.as_nanos() as u64;
    TIMELINE.data.retain(|v| v.timestamp > expire);
  }
}

pub fn ten_second_packet_dump(ip: XdpIpAddress) -> Vec<PacketHeader> {
  TIMELINE
    .data
    .iter()
    .filter(|e| e.src == ip || e.dst == ip)
    .map(|e| e.as_header())
    .collect()
}
