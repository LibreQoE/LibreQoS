/*
{"kind":"fq_codel","handle":"0:","parent":"7fff:a",
    "options":{"limit":10240,"flows":1024,"quantum":1514,"target":4999,"interval":99999,"memory_limit":33554432,"ecn":true,"drop_batch":64},
    "bytes":560,"packets":8,"drops":0,"overlimits":0,"requeues":0,"backlog":0,"qlen":0,"maxpacket":0,"drop_overlimit":0,"new_flow_count":0,
    "ecn_mark":0,"new_flows_len":0,"old_flows_len":0},
*/

use anyhow::{Result, Error};
use lqos_bus::TcHandle;
use serde::Serialize;
use serde_json::Value;

#[derive(Default, Clone, Debug, Serialize)]
pub(crate) struct TcFqCodel {
    handle: TcHandle,
    pub(crate) parent: TcHandle,
    options: TcFqCodelOptions,
    bytes: u64,
    packets: u64,
    drops: u64,
    overlimits: u64,
    requeues: u64,
    backlog: u64,
    qlen: u64,
    maxpacket: u64,
    drop_overlimit: u64,
    new_flow_count: u64,
    ecn_mark: u64,
    new_flows_len: u64,
    old_flows_len: u64,
}

#[derive(Default, Clone, Debug, Serialize)]
struct TcFqCodelOptions {
    limit: u64,
    flows: u64,
    quantum: u64,
    target: u64,
    interval: u64,
    memory_limit: u64,
    ecn: bool,
    drop_batch: u64,
}

impl TcFqCodel {
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
                "maxpacket" => result.maxpacket = value.as_u64().unwrap(),
                "drop_overlimit" => result.drop_overlimit = value.as_u64().unwrap(),
                "new_flow_count" => result.new_flow_count = value.as_u64().unwrap(),
                "ecn_mark" => result.ecn_mark = value.as_u64().unwrap(),
                "new_flows_len" => result.new_flows_len = value.as_u64().unwrap(),
                "old_flows_len" => result.old_flows_len = value.as_u64().unwrap(),
                "options" => result.options = TcFqCodelOptions::from_json(value)?,
                "kind" => {},
                _ => {
                    log::error!("Unknown entry in Tc-codel: {key}");
                }
            }
        }
        Ok(result)
    }
}

impl TcFqCodelOptions {
    fn from_json(value: &Value) -> Result<Self> {
        match value {
            Value::Object(map) => {
                let mut result = Self::default();
                for (key, value) in map.iter() {
                    match key.as_str() {
                        "limit" => result.limit = value.as_u64().unwrap(),
                        "flows" => result.flows = value.as_u64().unwrap(),
                        "quantum" => result.quantum = value.as_u64().unwrap(),
                        "target" => result.target = value.as_u64().unwrap(),
                        "interval" => result.interval = value.as_u64().unwrap(),
                        "memory_limit" => result.memory_limit = value.as_u64().unwrap(),
                        "ecn" => result.ecn = value.as_bool().unwrap(),
                        "drop_batch" => result.drop_batch = value.as_u64().unwrap(),
                        _ => {
                            log::error!("Unknown entry in Tc-codel-options: {key}");
                        }
                    }
                }
                Ok(result)
            }
            _ => Err(Error::msg("Unable to parse fq_codel options")),
        }        
    }
}