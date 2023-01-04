/*
{"kind":"mq","handle":"7fff:","root":true,"options":{},"bytes":0,"packets":0,"drops":0,"overlimits":0,"requeues":0,"backlog":0,"qlen":0}
*/

use lqos_bus::TcHandle;
use serde::Serialize;
use serde_json::Value;
use anyhow::Result;

#[derive(Default, Clone, Debug, Serialize)]
pub(crate) struct TcMultiQueue {
    handle: TcHandle,
    root: bool,
    bytes: u64,
    packets: u64,
    drops: u64,
    overlimits: u64,
    requeues: u64,
    backlog: u64,
    qlen: u64,
}

impl TcMultiQueue {
    pub(crate) fn from_json(map: &serde_json::Map<std::string::String, Value>) -> Result<Self> {
        let mut result = Self::default();
        for (key, value) in map.iter() {
            match key.as_str() {
                "handle" => result.handle = TcHandle::from_string(value.as_str().unwrap())?,
                "root" => result.root = value.as_bool().unwrap(),
                "bytes" => result.bytes = value.as_u64().unwrap(),
                "packets" => result.packets = value.as_u64().unwrap(),
                "drops" => result.drops = value.as_u64().unwrap(),
                "overlimits" => result.overlimits = value.as_u64().unwrap(),
                "requeues" => result.requeues = value.as_u64().unwrap(),
                "backlog" => result.backlog = value.as_u64().unwrap(),
                "qlen" => result.qlen = value.as_u64().unwrap(),
                "kind" => {},
                "options" => {},
                _ => {
                    log::error!("Unknown entry in Tc-MQ: {key}");
                }
            }
        }
        Ok(result)
    }
}