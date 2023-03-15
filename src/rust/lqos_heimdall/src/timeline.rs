use std::time::Duration;
use dashmap::DashSet;
use lqos_bus::{PacketHeader, tos_parser};
use lqos_utils::{unix_time::time_since_boot, XdpIpAddress};
use once_cell::sync::Lazy;
use zerocopy::AsBytes;
use crate::{perf_interface::{HeimdallEvent, PACKET_OCTET_SIZE}, pcap::{PcapFileHeader, PcapPacketHeader}};

impl HeimdallEvent {
    fn as_header(&self) -> PacketHeader {
      let (dscp, ecn) = tos_parser(self.tos);
      PacketHeader {
            timestamp: self.timestamp,
            src: self.src.as_ip().to_string(),
            dst: self.dst.as_ip().to_string(),
            src_port: self.src_port,
            dst_port: self.dst_port,
            ip_protocol: self.ip_protocol,
            ecn, dscp,
            size: self.size,
            tcp_flags: self.tcp_flags,
            tcp_window: self.tcp_window,
            tcp_tsecr: self.tcp_tsecr,
            tcp_tsval: self.tcp_tsval,
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

pub fn ten_second_pcap() -> Vec<u8> {
  let mut bytes : Vec<u8> = Vec::new();
  let file_header = PcapFileHeader::new();
  bytes.extend(file_header.as_bytes());
  let mut packets: Vec<HeimdallEvent> = TIMELINE.data.iter().map(|e| e.clone()).collect();
  packets.sort_by(|a,b| a.timestamp.cmp(&b.timestamp));
  packets.iter().for_each(|p| {
    let packet_header = PcapPacketHeader::from_heimdall(p);
    bytes.extend(packet_header.as_bytes());
    if p.size < PACKET_OCTET_SIZE as u32 {
      bytes.extend(&p.packet_data[0 .. p.size as usize]);
    } else {
      bytes.extend(p.packet_data);
    }
  });
  bytes
}