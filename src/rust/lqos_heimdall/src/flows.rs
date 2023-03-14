use crate::{perf_interface::HeimdallEvent, timeline::expire_timeline};
use dashmap::DashMap;
use lqos_bus::{tos_parser, BusResponse, FlowTransport};
use lqos_utils::{unix_time::time_since_boot, XdpIpAddress};
use once_cell::sync::Lazy;
use std::{collections::HashSet, time::Duration};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct FlowKey {
  src: XdpIpAddress,
  dst: XdpIpAddress,
  proto: u8,
  src_port: u16,
  dst_port: u16,
}

#[derive(Clone, Debug)]
struct FlowData {
  last_seen: u64,
  bytes: u64,
  packets: u64,
  tos: u8,
}

impl From<&HeimdallEvent> for FlowKey {
  fn from(value: &HeimdallEvent) -> Self {
    Self {
      src: value.src,
      dst: value.dst,
      proto: value.ip_protocol,
      src_port: value.src_port,
      dst_port: value.dst_port,
    }
  }
}

static FLOW_DATA: Lazy<DashMap<FlowKey, FlowData>> = Lazy::new(DashMap::new);

pub(crate) fn record_flow(event: &HeimdallEvent) {
  let key: FlowKey = event.into();
  if let Some(mut data) = FLOW_DATA.get_mut(&key) {
    data.last_seen = event.timestamp;
    data.packets += 1;
    data.bytes += event.size as u64;
    data.tos = event.tos;
  } else {
    FLOW_DATA.insert(
      key,
      FlowData {
        last_seen: event.timestamp,
        bytes: event.size.into(),
        packets: 1,
        tos: event.tos,
      },
    );
  }
}

pub fn expire_heimdall_flows() {
  if let Ok(now) = time_since_boot() {
    let since_boot = Duration::from(now);
    let thirty_secs_ago = since_boot - Duration::from_secs(30);
    let expire = thirty_secs_ago.as_nanos() as u64;
    FLOW_DATA.retain(|_k, v| v.last_seen > expire);
    expire_timeline();
  }
}

pub fn get_flow_stats(ip: XdpIpAddress) -> BusResponse {
  let mut result = Vec::new();

  // Obtain all the flows
  let mut all_flows = Vec::new();
  for value in FLOW_DATA.iter() {
    let key = value.key();
    if key.src == ip || key.dst == ip {
      let (dscp, ecn) = tos_parser(value.tos);
      all_flows.push(FlowTransport {
        src: key.src.as_ip().to_string(),
        dst: key.dst.as_ip().to_string(),
        src_port: key.src_port,
        dst_port: key.dst_port,
        proto: match key.proto {
          6 => lqos_bus::FlowProto::TCP,
          17 => lqos_bus::FlowProto::UDP,
          _ => lqos_bus::FlowProto::ICMP,
        },
        bytes: value.bytes,
        packets: value.packets,
        dscp,
        ecn,
      });
    }
  }

  // Turn them into reciprocal pairs
  let mut done = HashSet::new();
  for (i, flow) in all_flows.iter().enumerate() {
    if !done.contains(&i) {
      let flow_a = flow.clone();
      let flow_b = if let Some(flow_b) = all_flows
        .iter()
        .position(|f| f.src == flow_a.dst && f.src_port == flow_a.dst_port)
      {
        done.insert(flow_b);
        Some(all_flows[flow_b].clone())
      } else {
        None
      };

      result.push((flow_a, flow_b));
    }
  }

  result.sort_by(|a, b| b.0.bytes.cmp(&a.0.bytes));

  BusResponse::FlowData(result)
}
