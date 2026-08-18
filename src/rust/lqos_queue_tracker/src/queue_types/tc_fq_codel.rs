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
    pub(crate) handle: TcHandle,
    pub(crate) parent: TcHandle,
    pub(crate) options: TcFqCodelOptions,
    pub(crate) bytes: u64,
    pub(crate) packets: u64,
    pub(crate) drops: u64,
    pub(crate) overlimits: u64,
    pub(crate) requeues: u64,
    pub(crate) backlog: u64,
    pub(crate) qlen: u64,
    pub(crate) maxpacket: u64,
    pub(crate) drop_overlimit: u64,
    pub(crate) new_flow_count: u64,
    pub(crate) ecn_mark: u64,
    pub(crate) new_flows_len: u64,
    pub(crate) old_flows_len: u64,
}

#[derive(Default, Clone, Debug, Serialize)]
pub(crate) struct TcFqCodelOptions {
    pub(crate) limit: u64,
    pub(crate) flows: u64,
    pub(crate) quantum: u64,
    pub(crate) target: u64, // FIXME target and interval within fq_codel are scaled to ns >> 1024
    pub(crate) interval: u64, // tc scales them back up to us. Ideally ns would make sense throughout.
    pub(crate) memory_limit: u64,
    pub(crate) ecn: bool,
    pub(crate) drop_batch: u64, // FIXME CE_threshold is presently missing from the parser
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
                "packets" => result.packets = value.as_u64().unwrap_or(0),
                "drops" => result.drops = value.as_u64().unwrap_or(0),
                "overlimits" => result.overlimits = value.as_u64().unwrap_or(0),
                "requeues" => result.requeues = value.as_u64().unwrap_or(0),
                "backlog" => result.backlog = value.as_u64().unwrap_or(0),
                "qlen" => result.qlen = value.as_u64().unwrap_or(0),
                "maxpacket" => result.maxpacket = value.as_u64().unwrap_or(0),
                "drop_overlimit" => result.drop_overlimit = value.as_u64().unwrap_or(0),
                "new_flow_count" => result.new_flow_count = value.as_u64().unwrap_or(0),
                "ecn_mark" => result.ecn_mark = value.as_u64().unwrap_or(0),
                "new_flows_len" => result.new_flows_len = value.as_u64().unwrap_or(0),
                "old_flows_len" => result.old_flows_len = value.as_u64().unwrap_or(0),
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
                        "limit" => result.limit = value.as_u64().unwrap_or(0),
                        "flows" => result.flows = value.as_u64().unwrap_or(0),
                        "quantum" => result.quantum = value.as_u64().unwrap_or(0),
                        "target" => result.target = value.as_u64().unwrap_or(0),
                        "interval" => result.interval = value.as_u64().unwrap_or(0),
                        "memory_limit" => result.memory_limit = value.as_u64().unwrap_or(0),
                        "ecn" => result.ecn = value.as_bool().unwrap_or(false),
                        "drop_batch" => result.drop_batch = value.as_u64().unwrap_or(0),
                        _ => {
                            info!("Unknown entry in tc-codel-options json decoder: {key}");
                        }
                    }
                }
                Ok(result)
            }
            _ => Err(QDiscError::CodelOpts),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fq_codel_parser_preserves_large_kernel_counters() {
        let value = serde_json::json!({
            "kind": "fq_codel",
            "handle": "0:",
            "parent": "7fff:a",
            "options": {
                "limit": u64::from(u32::MAX) + 1,
                "flows": u64::from(u16::MAX) + 2,
                "quantum": u64::from(u16::MAX) + 3,
                "memory_limit": u64::from(u32::MAX) + 4,
                "drop_batch": u64::from(u16::MAX) + 5
            },
            "bytes": u64::from(u32::MAX) + 6,
            "packets": u64::from(u32::MAX) + 7,
            "drops": u64::from(u32::MAX) + 8,
            "overlimits": u64::from(u32::MAX) + 9,
            "requeues": u64::from(u32::MAX) + 10,
            "backlog": u64::from(u32::MAX) + 11,
            "qlen": u64::from(u32::MAX) + 12,
            "drop_overlimit": u64::from(u32::MAX) + 13,
            "new_flow_count": u64::from(u32::MAX) + 14,
            "ecn_mark": u64::from(u32::MAX) + 15,
            "new_flows_len": u64::from(u16::MAX) + 16,
            "old_flows_len": u64::from(u16::MAX) + 17
        });
        let Value::Object(map) = value else {
            panic!("test fixture should be a JSON object");
        };

        let parsed = TcFqCodel::from_json(&map).expect("fq_codel fixture should parse");

        assert_eq!(parsed.options.flows, u64::from(u16::MAX) + 2);
        assert_eq!(parsed.options.memory_limit, u64::from(u32::MAX) + 4);
        assert_eq!(parsed.packets, u64::from(u32::MAX) + 7);
        assert_eq!(parsed.drops, u64::from(u32::MAX) + 8);
        assert_eq!(parsed.backlog, u64::from(u32::MAX) + 11);
        assert_eq!(parsed.drop_overlimit, u64::from(u32::MAX) + 13);
        assert_eq!(parsed.ecn_mark, u64::from(u32::MAX) + 15);
        assert_eq!(parsed.new_flows_len, u64::from(u16::MAX) + 16);
    }
}
