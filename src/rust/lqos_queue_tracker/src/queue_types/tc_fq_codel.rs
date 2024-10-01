/*
{"kind":"fq_codel","handle":"0:","parent":"7fff:a",
    "options":{"limit":10240,"flows":1024,"quantum":1514,"target":4999,"interval":99999,"memory_limit":33554432,"ecn":true,"drop_batch":64},
    "bytes":560,"packets":8,"drops":0,"overlimits":0,"requeues":0,"backlog":0,"qlen":0,"maxpacket":0,"drop_overlimit":0,"new_flow_count":0,
    "ecn_mark":0,"new_flows_len":0,"old_flows_len":0},
*/

use super::QDiscError;
use crate::parse_tc_handle;
use lqos_bus::TcHandle;
use serde::Serialize;
use serde_json::Value;
use tracing::info;

#[derive(Default, Clone, Debug, Serialize)]
pub struct TcFqCodel {
  handle: TcHandle,
  pub(crate) parent: TcHandle,
  options: TcFqCodelOptions,
  bytes: u64,
  packets: u32, // FIXME - for long term data we have to worry about wrapping
  drops: u32,
  overlimits: u32,
  requeues: u32,
  backlog: u32,
  qlen: u32,
  maxpacket: u16,
  drop_overlimit: u32,
  new_flow_count: u32,
  ecn_mark: u32,
  new_flows_len: u16,
  old_flows_len: u16,
}

#[derive(Default, Clone, Debug, Serialize)]
struct TcFqCodelOptions {
  limit: u32,
  flows: u16,
  quantum: u16,
  target: u64, // FIXME target and interval within fq_codel are scaled to ns >> 1024
  interval: u64, // tc scales them back up to us. Ideally ns would make sense throughout.
  memory_limit: u32,
  ecn: bool,
  drop_batch: u16, // FIXME CE_threshold is presently missing from the parser
}

impl TcFqCodel {
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
        "drops" => result.drops = value.as_u64().unwrap_or(0) as u32,
        "overlimits" => result.overlimits = value.as_u64().unwrap_or(0) as u32,
        "requeues" => result.requeues = value.as_u64().unwrap_or(0) as u32,
        "backlog" => result.backlog = value.as_u64().unwrap_or(0) as u32,
        "qlen" => result.qlen = value.as_u64().unwrap_or(0) as u32,
        "maxpacket" => result.maxpacket = value.as_u64().unwrap_or(0) as u16,
        "drop_overlimit" => {
          result.drop_overlimit = value.as_u64().unwrap_or(0) as u32
        }
        "new_flow_count" => {
          result.new_flow_count = value.as_u64().unwrap_or(0) as u32
        }
        "ecn_mark" => result.ecn_mark = value.as_u64().unwrap_or(0) as u32,
        "new_flows_len" => {
          result.new_flows_len = value.as_u64().unwrap_or(0) as u16
        }
        "old_flows_len" => {
          result.old_flows_len = value.as_u64().unwrap_or(0) as u16
        }
        "options" => result.options = TcFqCodelOptions::from_json(value)?,
        "kind" => {}
        _ => {
          info!("Unknown entry in tc-codel json decoder: {key}");
        }
      }
    }
    Ok(result)
  }
}

impl TcFqCodelOptions {
  fn from_json(value: &Value) -> Result<Self, QDiscError> {
    match value {
      Value::Object(map) => {
        let mut result = Self::default();
        for (key, value) in map.iter() {
          match key.as_str() {
            "limit" => result.limit = value.as_u64().unwrap_or(0) as u32,
            "flows" => result.flows = value.as_u64().unwrap_or(0) as u16,
            "quantum" => result.quantum = value.as_u64().unwrap_or(0) as u16,
            "target" => result.target = value.as_u64().unwrap_or(0),
            "interval" => result.interval = value.as_u64().unwrap_or(0),
            "memory_limit" => {
              result.memory_limit = value.as_u64().unwrap_or(0) as u32
            }
            "ecn" => result.ecn = value.as_bool().unwrap_or(false),
            "drop_batch" => {
              result.drop_batch = value.as_u64().unwrap_or(0) as u16
            }
            _ => {
              info!(
                "Unknown entry in tc-codel-options json decoder: {key}"
              );
            }
          }
        }
        Ok(result)
      }
      _ => Err(QDiscError::CodelOpts),
    }
  }
}
