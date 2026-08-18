/*
{"kind":"mq","handle":"7fff:","root":true,"options":{},"bytes":0,"packets":0,"drops":0,"overlimits":0,"requeues":0,"backlog":0,"qlen":0}
*/

use super::QDiscError;
use crate::parse_tc_handle;
use lqos_bus::TcHandle;
use serde::Serialize;
use serde_json::Value;
use tracing::info;

#[derive(Default, Clone, Debug, Serialize)]
pub struct TcMultiQueue {
    handle: TcHandle,
    root: bool,
    bytes: u64,
    packets: u64,
    drops: u64,
    overlimits: u64,
    requeues: u64, // what does requeues really mean?
    backlog: u64,
    qlen: u64,
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
                "packets" => result.packets = value.as_u64().unwrap_or(0),
                "drops" => result.drops = value.as_u64().unwrap_or(0),
                "overlimits" => result.overlimits = value.as_u64().unwrap_or(0),
                "requeues" => result.requeues = value.as_u64().unwrap_or(0),
                "backlog" => result.backlog = value.as_u64().unwrap_or(0),
                "qlen" => result.qlen = value.as_u64().unwrap_or(0),
                "kind" => {}
                "options" => {}
                _ => {
                    info!("Unknown entry in tc-MQ json decoder: {key}");
                }
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mq_parser_preserves_large_kernel_counters() {
        let value = serde_json::json!({
            "kind": "mq",
            "handle": "7fff:",
            "root": true,
            "options": {},
            "bytes": u64::from(u32::MAX) + 1,
            "packets": u64::from(u32::MAX) + 2,
            "drops": u64::from(u32::MAX) + 3,
            "overlimits": u64::from(u32::MAX) + 4,
            "requeues": u64::from(u32::MAX) + 5,
            "backlog": u64::from(u32::MAX) + 6,
            "qlen": u64::from(u32::MAX) + 7
        });
        let Value::Object(map) = value else {
            panic!("test fixture should be a JSON object");
        };

        let parsed = TcMultiQueue::from_json(&map).expect("mq fixture should parse");

        assert_eq!(parsed.packets, u64::from(u32::MAX) + 2);
        assert_eq!(parsed.drops, u64::from(u32::MAX) + 3);
        assert_eq!(parsed.backlog, u64::from(u32::MAX) + 6);
        assert_eq!(parsed.qlen, u64::from(u32::MAX) + 7);
    }
}
