use super::QDiscError;
use crate::parse_tc_handle;
use tracing::{error, info, warn};
use lqos_bus::TcHandle;
use lqos_utils::{dashy_table_enum, string_table_enum};
use serde::{Deserialize, Serialize};
use serde_json::Value;

string_table_enum!(
  DiffServ, besteffort, diffserv3, diffserv4, diffserv8, precedence
);
dashy_table_enum!(AckFilter, none, ack_filter, ack_filter_aggressive);
dashy_table_enum!(
  FlowMode,
  flowblind,
  srchost,
  dsthost,
  hosts,
  dual_srchost,
  dual_dsthost,
  triple_isolate
);
string_table_enum!(BandWidth, unlimited); // in the present implementation with htb, always unlimited

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct TcCake {
  pub(crate) handle: TcHandle,
  pub(crate) parent: TcHandle,
  pub(crate) options: TcCakeOptions,
  pub(crate) bytes: u64,
  pub(crate) packets: u32,
  pub(crate) overlimits: u32,
  pub(crate) requeues: u32,
  pub(crate) backlog: u32,
  pub(crate) qlen: u32,
  pub(crate) memory_used: u32,
  pub(crate) memory_limit: u32,
  pub(crate) capacity_estimate: u32,
  pub(crate) min_network_size: u16,
  pub(crate) max_network_size: u16,
  pub(crate) min_adj_size: u16,
  pub(crate) max_adj_size: u16,
  pub(crate) avg_hdr_offset: u16,
  pub(crate) tins: Vec<TcCakeTin>,
  pub(crate) drops: u32,
}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub(crate) struct TcCakeOptions {
  pub(crate) rtt: u64,
  pub(crate) bandwidth: BandWidth,
  pub(crate) diffserv: DiffServ,
  pub(crate) flowmode: FlowMode,
  pub(crate) ack_filter: AckFilter,
  pub(crate) nat: bool,
  pub(crate) wash: bool,
  pub(crate) ingress: bool,
  pub(crate) split_gso: bool,
  pub(crate) raw: bool,
  pub(crate) overhead: u16,
  pub(crate) fwmark: TcHandle,
}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub(crate) struct TcCakeTin {
  pub(crate) threshold_rate: u64,
  pub(crate) sent_bytes: u64,
  pub(crate) backlog_bytes: u32,
  pub(crate) target_us: u32,
  pub(crate) interval_us: u32,
  pub(crate) peak_delay_us: u32,
  pub(crate) avg_delay_us: u32,
  pub(crate) base_delay_us: u32,
  pub(crate) sent_packets: u32,
  pub(crate) way_indirect_hits: u16,
  pub(crate) way_misses: u16,
  pub(crate) way_collisions: u16,
  pub(crate) drops: u32,
  pub(crate) ecn_marks: u32,
  pub(crate) ack_drops: u32,
  pub(crate) sparse_flows: u16,
  pub(crate) bulk_flows: u16,
  pub(crate) unresponsive_flows: u16,
  pub(crate) max_pkt_len: u16,
  pub(crate) flow_quantum: u16,
}

impl TcCake {
  pub(crate) fn from_json(
    map: &serde_json::Map<std::string::String, Value>,
  ) -> Result<Self, QDiscError> {
    let mut result = Self::default();
    for (key, value) in map.iter() {
      match key.as_str() {
        "handle" => {
          parse_tc_handle!(result.handle, value);
        }
        "parent" => {
          parse_tc_handle!(result.parent, value);
        }
        "bytes" => result.bytes = value.as_u64().unwrap_or(0),
        "packets" => result.packets = value.as_u64().unwrap_or(0) as u32,
        "overlimits" => result.overlimits = value.as_u64().unwrap_or(0) as u32,
        "requeues" => result.requeues = value.as_u64().unwrap_or(0) as u32,
        "backlog" => result.backlog = value.as_u64().unwrap_or(0) as u32,
        "qlen" => result.qlen = value.as_u64().unwrap_or(0) as u32,
        "memory_used" => {
          result.memory_used = value.as_u64().unwrap_or(0) as u32
        }
        "memory_limit" => {
          result.memory_limit = value.as_u64().unwrap_or(0) as u32
        }
        "capacity_estimate" => {
          result.capacity_estimate = value.as_u64().unwrap_or(0) as u32
        }
        "min_network_size" => {
          result.min_network_size = value.as_u64().unwrap_or(0) as u16
        }
        "max_network_size" => {
          result.max_network_size = value.as_u64().unwrap_or(0) as u16
        }
        "min_adj_size" => {
          result.min_adj_size = value.as_u64().unwrap_or(0) as u16
        }
        "max_adj_size" => {
          result.max_adj_size = value.as_u64().unwrap_or(0) as u16
        }
        "avg_hdr_offset" => {
          result.avg_hdr_offset = value.as_u64().unwrap_or(0) as u16
        }
        "drops" => result.drops = value.as_u64().unwrap_or(0) as u32,
        "options" => result.options = TcCakeOptions::from_json(value)?,
        "tins" => {
          if let Value::Array(array) = value {
            for value in array.iter() {
              result.tins.push(TcCakeTin::from_json(value)?);
            }
          }
        }
        "kind" => {}
        _ => {
          error!("Unknown entry in Tc-cake: {key}");
        }
      }
    }
    Ok(result)
  }
}

impl TcCakeOptions {
  fn from_json(value: &Value) -> Result<Self, QDiscError> {
    match value {
      Value::Object(map) => {
        let mut result = Self::default();
        for (key, value) in map.iter() {
          match key.as_str() {
            "bandwidth" => {
              result.bandwidth =
                BandWidth::from_str(value.as_str().unwrap_or(""))
            }
            "diffserv" => {
              result.diffserv =
                DiffServ::from_str(value.as_str().unwrap_or(""))
            }
            "flowmode" => {
              result.flowmode =
                FlowMode::from_str(value.as_str().unwrap_or(""))
            }
            "nat" => result.nat = value.as_bool().unwrap_or(false),
            "wash" => result.wash = value.as_bool().unwrap_or(false),
            "ingress" => result.ingress = value.as_bool().unwrap_or(false),
            "ack-filter" => {
              result.ack_filter =
                AckFilter::from_str(value.as_str().unwrap_or(""))
            }
            "split_gso" => result.split_gso = value.as_bool().unwrap_or(false),
            "rtt" => result.rtt = value.as_u64().unwrap_or(0),
            "raw" => result.raw = value.as_bool().unwrap_or(false),
            "overhead" => result.overhead = value.as_u64().unwrap_or(0) as u16,
            "fwmark" => {
              parse_tc_handle!(result.fwmark, value);
            }
            _ => {
              info!(
                "Unknown entry in tc-cake-options json decoder: {key}"
              );
            }
          }
        }
        Ok(result)
      }
      _ => Err(QDiscError::CakeOpts),
    }
  }
}

impl TcCakeTin {
  fn from_json(value: &Value) -> Result<Self, QDiscError> {
    match value {
      Value::Object(map) => {
        let mut result = Self::default();
        for (key, value) in map.iter() {
          match key.as_str() {
            "threshold_rate" => {
              result.threshold_rate = value.as_u64().unwrap_or(0)
            }
            "sent_bytes" => result.sent_bytes = value.as_u64().unwrap_or(0),
            "backlog_bytes" => {
              result.backlog_bytes = value.as_u64().unwrap_or(0) as u32
            }
            "target_us" => {
              result.target_us = value.as_u64().unwrap_or(0) as u32
            }
            "interval_us" => {
              result.interval_us = value.as_u64().unwrap_or(0) as u32
            }
            "peak_delay_us" => {
              result.peak_delay_us = value.as_u64().unwrap_or(0) as u32
            }
            "avg_delay_us" => {
              result.avg_delay_us = value.as_u64().unwrap_or(0) as u32
            }
            "base_delay_us" => {
              result.base_delay_us = value.as_u64().unwrap_or(0) as u32
            }
            "sent_packets" => {
              result.sent_packets = value.as_u64().unwrap_or(0) as u32
            }
            "way_indirect_hits" => {
              result.way_indirect_hits = value.as_u64().unwrap_or(0) as u16
            }
            "way_misses" => {
              result.way_misses = value.as_u64().unwrap_or(0) as u16
            }
            "way_collisions" => {
              result.way_collisions = value.as_u64().unwrap_or(0) as u16
            }
            "drops" => result.drops = value.as_u64().unwrap_or(0) as u32,
            "ecn_mark" => {
              result.ecn_marks = value.as_u64().unwrap_or(0) as u32
            }
            "ack_drops" => {
              result.ack_drops = value.as_u64().unwrap_or(0) as u32
            }
            "sparse_flows" => {
              result.sparse_flows = value.as_u64().unwrap_or(0) as u16
            }
            "bulk_flows" => {
              result.bulk_flows = value.as_u64().unwrap_or(0) as u16
            }
            "unresponsive_flows" => {
              result.unresponsive_flows = value.as_u64().unwrap_or(0) as u16
            }
            "max_pkt_len" => {
              result.max_pkt_len = value.as_u64().unwrap_or(0) as u16
            }
            "flow_quantum" => {
              result.flow_quantum = value.as_u64().unwrap_or(0) as u16
            }
            _ => {
              info!("Unknown entry in tc-cake-tin json decoder: {key}");
            }
          }
        }
        Ok(result)
      }
      _ => {
        warn!("Unable to parse cake tin");
        Err(QDiscError::CakeTin)
      }
    }
  }
}

// Example data

/*

{
    "kind": "cake",
    "handle": "9cb1:",
    "parent": "3:205",
    "options": {
        "bandwidth": "unlimited",
        "diffserv": "diffserv4",
        "flowmode": "triple-isolate",
        "nat": false,
        "wash": false,
        "ingress": false,
        "ack-filter": "disabled",
        "split_gso": true,
        "rtt": 100000,
        "raw": true,
        "overhead": 0,
        "fwmark": "0"
    },
    "bytes": 49072087981,
    "packets": 35792920,
    "drops": 1162331,
    "overlimits": 0,
    "requeues": 0,
    "backlog": 0,
    "qlen": 0,
    "memory_used": 2002176,
    "memory_limit": 15503360,
    "capacity_estimate": 0,
    "min_network_size": 56,
    "max_network_size": 1514,
    "min_adj_size": 56,
    "max_adj_size": 1514,
    "avg_hdr_offset": 14,
    "tins": [
        {
            "threshold_rate": 0,
            "sent_bytes": 0,
            "backlog_bytes": 0,
            "target_us": 5000,
            "interval_us": 100000,
            "peak_delay_us": 0,
            "avg_delay_us": 0,
            "base_delay_us": 0,
            "sent_packets": 0,
            "way_indirect_hits": 0,
            "way_misses": 0,
            "way_collisions": 0,
            "drops": 0,
            "ecn_mark": 0,
            "ack_drops": 0,
            "sparse_flows": 0,
            "bulk_flows": 0,
            "unresponsive_flows": 0,
            "max_pkt_len": 0,
            "flow_quantum": 1514
        },
        {
            "threshold_rate": 0,
            "sent_bytes": 47096460394,
            "backlog_bytes": 0,
            "target_us": 5000,
            "interval_us": 100000,
            "peak_delay_us": 152,
            "avg_delay_us": 7,
            "base_delay_us": 1,
            "sent_packets": 34376628,
            "way_indirect_hits": 156580,
            "way_misses": 89285,
            "way_collisions": 0,
            "drops": 984524,
            "ecn_mark": 10986,
            "ack_drops": 0,
            "sparse_flows": 1,
            "bulk_flows": 0,
            "unresponsive_flows": 0,
            "max_pkt_len": 1514,
            "flow_quantum": 1514
        },
        {
            "threshold_rate": 0,
            "sent_bytes": 3481013747,
            "backlog_bytes": 0,
            "target_us": 5000,
            "interval_us": 100000,
            "peak_delay_us": 1080,
            "avg_delay_us": 141,
            "base_delay_us": 1,
            "sent_packets": 2456582,
            "way_indirect_hits": 282,
            "way_misses": 3916,
            "way_collisions": 0,
            "drops": 177080,
            "ecn_mark": 25,
            "ack_drops": 0,
            "sparse_flows": 0,
            "bulk_flows": 0,
            "unresponsive_flows": 0,
            "max_pkt_len": 1514,
            "flow_quantum": 1514
        },
        {
            "threshold_rate": 0,
            "sent_bytes": 145417781,
            "backlog_bytes": 0,
            "target_us": 5000,
            "interval_us": 100000,
            "peak_delay_us": 566715,
            "avg_delay_us": 421103,
            "base_delay_us": 3,
            "sent_packets": 122041,
            "way_indirect_hits": 11,
            "way_misses": 148,
            "way_collisions": 0,
            "drops": 727,
            "ecn_mark": 0,
            "ack_drops": 0,
            "sparse_flows": 2,
            "bulk_flows": 0,
            "unresponsive_flows": 0,
            "max_pkt_len": 1242,
            "flow_quantum": 1514
        }
    ]
},

 */
