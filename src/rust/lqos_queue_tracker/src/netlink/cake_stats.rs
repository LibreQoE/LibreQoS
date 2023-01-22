use zerocopy::FromBytes;

use super::nla_types::{Nla64, Nla32};

#[repr(C, packed)]
#[derive(Copy, Clone, FromBytes)]
pub struct CakeStatsDiffServ4 {
  capacity: Nla64,
  memory_limit: Nla32,
  memory_used: Nla32,
  avg_netoff: Nla32,
  max_netlen: Nla32,
  max_adjlen: Nla32,
  min_netlen: Nla32,
  min_adjlen: Nla32,

  unknown_length: u16,
  unknown_type: u16,

  padding: [u8; 16],
  tins: [CakeTin; 4],
}

#[repr(C, packed)]
#[derive(Copy, Clone, FromBytes)]
pub struct CakeTin {
  threshold_rate64: Nla64,
  sent_bytes64: Nla64,
  backlog_bytes: Nla32,
  target_us: Nla32,
  interval_us: Nla32,
  sent_packets: Nla32,
  dropped_packets: Nla32,
  ecn_marked_packets: Nla32,
  acks_dropped_packets: Nla32,
  peak_delay_us: Nla32,
  avg_delay_us: Nla32,
  base_delay_us: Nla32,
  way_indirect_hits: Nla32,
  way_missed: Nla32,
  way_collisions: Nla32,
  sparse_flows: Nla32,
  bulk_flows: Nla32,
  unresponsive_flows: Nla32,
  max_skblen: Nla32,
  flow_quantum: Nla32,  
}