/*
{"kind":"htb","handle":"2:","parent":"7fff:2","options":{"r2q":10,"default":"0x2","direct_packets_stat":7,"direct_qlen":1000},
"bytes":1920791512305,"packets":1466145855,"drops":32136937,"overlimits":2627500070,"requeues":1224,"backlog":0,"qlen":0}
*/

use anyhow::{Result, Error};
use lqos_bus::TcHandle;
use serde::Serialize;
use serde_json::Value;

#[derive(Default, Clone, Debug, Serialize)]
pub(crate) struct TcHtb {
    handle: TcHandle,
    parent: TcHandle,
    options: TcHtbOptions,
    bytes: u64,
    packets: u64,
    drops: u64,
    overlimits: u64,
    requeues: u64,
    backlog: u64,
    qlen: u64,
}

#[derive(Default, Clone, Debug, Serialize)]
struct TcHtbOptions {
    r2q: u64,
    default: TcHandle,
    direct_packets_stat: u64,
    direct_qlen: u64,
}

impl TcHtb {
    pub(crate) fn from_json(map: &serde_json::Map<std::string::String, Value>) -> Result<Self> {
        let mut result = Self::default();
        for (key, value) in map.iter() {
            match key.as_str() {
                "handle" => result.handle = TcHandle::from_string(value.as_str().unwrap())?,
                "parent" => result.parent = TcHandle::from_string(value.as_str().unwrap())?,
                "bytes" => result.bytes = value.as_u64().unwrap(),
                "packets" => result.packets = value.as_u64().unwrap(),
                "drops" => result.drops = value.as_u64().unwrap(),
                "overlimits" => result.overlimits = value.as_u64().unwrap(),
                "requeues" => result.requeues = value.as_u64().unwrap(),
                "backlog" => result.backlog = value.as_u64().unwrap(),
                "qlen" => result.qlen = value.as_u64().unwrap(),
                "options" => result.options = TcHtbOptions::from_json(value)?,
                "kind" => {},
                _ => {
                    log::error!("Unknown entry in Tc-HTB: {key}");
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
                        "r2q" => result.r2q = value.as_u64().unwrap(),
                        "default" => result.default = TcHandle::from_string(value.as_str().unwrap())?,
                        "direct_packets_stat" => result.direct_packets_stat = value.as_u64().unwrap(),
                        "direct_qlen" => result.direct_qlen = value.as_u64().unwrap(),
                        _ => {
                            log::error!("Unknown entry in Tc-HTB: {key}");
                        }
                    }
                }
                Ok(result)
            }
            _ => Err(Error::msg("Unable to parse HTB options")),
        }        
    }
}