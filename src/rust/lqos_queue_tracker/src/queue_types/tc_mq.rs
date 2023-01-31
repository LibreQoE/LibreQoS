/*
{"kind":"mq","handle":"7fff:","root":true,"options":{},"bytes":0,"packets":0,"drops":0,"overlimits":0,"requeues":0,"backlog":0,"qlen":0}
*/

use super::QDiscError;
use crate::parse_tc_handle;
use log_once::info_once;
use lqos_bus::TcHandle;
use serde::Serialize;
use serde_json::Value;

#[derive(Default, Clone, Debug, Serialize)]
pub struct TcMultiQueue {
  handle: TcHandle,
  root: bool,
  bytes: u64,
  packets: u32, // FIXME These can overflow in older linuxes
  drops: u32,
  overlimits: u32,
  requeues: u32, // what does requeues really mean?
  backlog: u32,
  qlen: u32,
}

impl TcMultiQueue {
  pub(crate) fn from_json(
    map: &serde_json::Map<std::string::String, Value>,
  ) -> Result<Self, QDiscError> {
    let mut result = Self::default();
    for (key, value) in map.iter() {
      match key.as_str() {
        "handle" => {
          parse_tc_handle!(result.handle, value);
        }
        "root" => result.root = value.as_bool().unwrap_or(false),
        "bytes" => result.bytes = value.as_u64().unwrap_or(0),
        "packets" => result.packets = value.as_u64().unwrap_or(0) as u32,
        "drops" => result.drops = value.as_u64().unwrap_or(0) as u32,
        "overlimits" => {
          result.overlimits = value.as_u64().unwrap_or(0) as u32
        }
        "requeues" => result.requeues = value.as_u64().unwrap_or(0) as u32,
        "backlog" => result.backlog = value.as_u64().unwrap_or(0) as u32,
        "qlen" => result.qlen = value.as_u64().unwrap_or(0) as u32,
        "kind" => {}
        "options" => {}
        _ => {
          info_once!("Unknown entry in tc-MQ json decoder: {key}");
        }
      }
    }
    Ok(result)
  }
}
