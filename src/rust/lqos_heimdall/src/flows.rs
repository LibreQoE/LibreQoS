use crate::{timeline::expire_timeline, FLOW_EXPIRE_SECS};
use dashmap::DashMap;
use lqos_bus::{tos_parser, BusResponse, FlowTransport};
use lqos_sys::bpf_per_cpu_map::BpfPerCpuMap;
use lqos_utils::{unix_time::time_since_boot, XdpIpAddress};
use once_cell::sync::Lazy;
use std::{collections::HashSet, time::Duration};

/// Representation of the eBPF `heimdall_key` type.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct HeimdallKey {
  /// Mapped `XdpIpAddress` source for the flow.
  pub src_ip: XdpIpAddress,
  /// Mapped `XdpIpAddress` destination for the flow
  pub dst_ip: XdpIpAddress,
  /// IP protocol (see the Linux kernel!)
  pub ip_protocol: u8,
  /// Source port number, or ICMP type.
  pub src_port: u16,
  /// Destination port number.
  pub dst_port: u16,
}

/// Mapped representation of the eBPF `heimdall_data` type.
#[derive(Debug, Clone, Default)]
#[repr(C)]
pub struct HeimdallData {
  /// Last seen, in nanoseconds (since boot time).
  pub last_seen: u64,
  /// Number of bytes since the flow started being tracked
  pub bytes: u64,
  /// Number of packets since the flow started being tracked
  pub packets: u64,
  /// IP header TOS value
  pub tos: u8,
  /// Reserved to pad the structure
  pub reserved: [u8; 3],
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct FlowKey {
  src: XdpIpAddress,
  dst: XdpIpAddress,
  proto: u8,
  src_port: u16,
  dst_port: u16,
}

#[derive(Clone, Debug, Default)]
struct FlowData {
  last_seen: u64,
  bytes: u64,
  packets: u64,
  tos: u8,
}

impl From<&HeimdallKey> for FlowKey {
  fn from(value: &HeimdallKey) -> Self {
    Self {
      src: value.src_ip,
      dst: value.dst_ip,
      proto: value.ip_protocol,
      src_port: value.src_port,
      dst_port: value.dst_port,
    }
  }
}

static FLOW_DATA: Lazy<DashMap<FlowKey, FlowData>> = Lazy::new(DashMap::new);

/*pub(crate) fn record_flow(event: &HeimdallEvent) {
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
}*/


/// Iterates through all throughput entries, and sends them in turn to `callback`.
/// This elides the need to clone or copy data.
fn heimdall_for_each(
  callback: &mut dyn FnMut(&HeimdallKey, &[HeimdallData]),
) {
  if let Ok(heimdall) = BpfPerCpuMap::<HeimdallKey, HeimdallData>::from_path(
    "/sys/fs/bpf/heimdall",
  ) {
    heimdall.for_each(callback);
  }
}


fn combine_flows(values: &[HeimdallData]) -> FlowData {
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

pub fn read_flows() {
  heimdall_for_each(&mut |key, value| {
    let flow_key = key.into();
    let combined = combine_flows(value);
    if let Some(mut flow) = FLOW_DATA.get_mut(&flow_key) {
      flow.last_seen = combined.last_seen;
      flow.bytes = combined.bytes;
      flow.packets = combined.packets;
      flow.tos = combined.tos;
    } else {
      FLOW_DATA.insert(flow_key, combined);
    }
  });
}

/// Expire flows that have not been seen in a while.
pub fn expire_heimdall_flows() {
  if let Ok(now) = time_since_boot() {
    let since_boot = Duration::from(now);
    let expire = (since_boot - Duration::from_secs(FLOW_EXPIRE_SECS)).as_nanos() as u64;
    FLOW_DATA.retain(|_k, v| v.last_seen > expire);
    expire_timeline();
  }
}

/// Get the flow stats for a given IP address.
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
