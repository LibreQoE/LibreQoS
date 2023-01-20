/*
{"kind":"htb","handle":"2:","parent":"7fff:2","options":{"r2q":10,"default":"0x2","direct_packets_stat":7,"direct_qlen":1000},
"bytes":1920791512305,"packets":1466145855,"drops":32136937,"overlimits":2627500070,"requeues":1224,"backlog":0,"qlen":0}
*/

use anyhow::{Error, Result};
use lqos_bus::TcHandle;
use serde::Serialize;
use serde_json::Value;
use log_once::info_once;

#[derive(Default, Clone, Debug, Serialize)]
pub struct TcHtb {
    handle: TcHandle,
    parent: TcHandle,
    bytes: u64,
    packets: u32,
    drops: u32,
    overlimits: u32,
    requeues: u32,
    backlog: u32,
    qlen: u32,
    options: TcHtbOptions,
}

#[derive(Default, Clone, Debug, Serialize)]
struct TcHtbOptions {
    default: TcHandle,
    r2q: u32,
    direct_qlen: u32,
    direct_packets_stat: u32,
}

impl TcHtb {
    pub(crate) fn from_json(map: &serde_json::Map<std::string::String, Value>) -> Result<Self> {
        let mut result = Self::default();
        for (key, value) in map.iter() {
            match key.as_str() {
                "handle" => result.handle = TcHandle::from_string(value.as_str().unwrap())?,
                "parent" => result.parent = TcHandle::from_string(value.as_str().unwrap())?,
                "bytes" => result.bytes = value.as_u64().unwrap(),
                "packets" => result.packets = value.as_u64().unwrap() as u32,
                "drops" => result.drops = value.as_u64().unwrap() as u32,
                "overlimits" => result.overlimits = value.as_u64().unwrap() as u32,
                "requeues" => result.requeues = value.as_u64().unwrap() as u32,
                "backlog" => result.backlog = value.as_u64().unwrap() as u32,
                "qlen" => result.qlen = value.as_u64().unwrap() as u32,
                "options" => result.options = TcHtbOptions::from_json(value)?,
                "kind" => {}
                _ => {
                    info_once!("Unknown entry in tc-HTB json decoder: {key}");
                }
            }
        }
        Ok(result)
    }
}

impl TcHtbOptions {
    fn from_json(value: &Value) -> Result<Self> {
        match value {
            Value::Object(map) => {
                let mut result = Self::default();
                for (key, value) in map.iter() {
                    match key.as_str() {
                        "r2q" => result.r2q = value.as_u64().unwrap() as u32,
                        "default" => {
                            result.default = TcHandle::from_string(value.as_str().unwrap())?
                        }
                        "direct_packets_stat" => {
                            result.direct_packets_stat = value.as_u64().unwrap() as u32
                        }
                        "direct_qlen" => result.direct_qlen = value.as_u64().unwrap() as u32,
                        _ => {
                            info_once!("Unknown entry in tc-HTB json decoder: {key}");
                        }
                    }
                }
                Ok(result)
            }
            _ => Err(Error::msg("Unable to parse HTB options")),
        }
    }
}
