/*
{"kind":"htb","handle":"2:","parent":"7fff:2","options":{"r2q":10,"default":"0x2","direct_packets_stat":7,"direct_qlen":1000},
"bytes":1920791512305,"packets":1466145855,"drops":32136937,"overlimits":2627500070,"requeues":1224,"backlog":0,"qlen":0}
*/

use super::QDiscError;
use crate::parse_tc_handle;
use lqos_bus::TcHandle;
use serde::Serialize;
use serde_json::Value;
use tracing::info;

#[derive(Default, Clone, Debug, Serialize)]
pub struct TcHtb {
    handle: TcHandle,
    parent: TcHandle,
    bytes: u64,
    packets: u64,
    drops: u64,
    overlimits: u64,
    requeues: u64,
    backlog: u64,
    qlen: u64,
    options: TcHtbOptions,
}

#[derive(Default, Clone, Debug, Serialize)]
struct TcHtbOptions {
    default: TcHandle,
    r2q: u64,
    direct_qlen: u64,
    direct_packets_stat: u64,
}

impl TcHtb {
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
                "options" => result.options = TcHtbOptions::from_json(value)?,
                "kind" => {}
                _ => {
                    info!("Unknown entry in tc-HTB json decoder: {key}");
                }
            }
        }
        Ok(result)
    }
}

impl TcHtbOptions {
    fn from_json(value: &Value) -> Result<Self, QDiscError> {
        match value {
            Value::Object(map) => {
                let mut result = Self::default();
                for (key, value) in map.iter() {
                    match key.as_str() {
                        "r2q" => result.r2q = value.as_u64().unwrap_or(0),
                        "default" => {
                            parse_tc_handle!(result.default, value);
                        }
                        "direct_packets_stat" => {
                            result.direct_packets_stat = value.as_u64().unwrap_or(0)
                        }
                        "direct_qlen" => result.direct_qlen = value.as_u64().unwrap_or(0),
                        _ => {
                            info!("Unknown entry in tc-HTB json decoder: {key}");
                        }
                    }
                }
                Ok(result)
            }
            _ => Err(QDiscError::HtbOpts),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn htb_parser_preserves_large_kernel_counters() {
        let value = serde_json::json!({
            "kind": "htb",
            "handle": "2:",
            "parent": "7fff:2",
            "options": {
                "direct_packets_stat": u64::from(u32::MAX) + 1,
                "direct_qlen": u64::from(u32::MAX) + 2
            },
            "bytes": u64::from(u32::MAX) + 3,
            "packets": u64::from(u32::MAX) + 4,
            "drops": u64::from(u32::MAX) + 5,
            "overlimits": u64::from(u32::MAX) + 6,
            "requeues": u64::from(u32::MAX) + 7,
            "backlog": u64::from(u32::MAX) + 8,
            "qlen": u64::from(u32::MAX) + 9
        });
        let Value::Object(map) = value else {
            panic!("test fixture should be a JSON object");
        };

        let parsed = TcHtb::from_json(&map).expect("htb fixture should parse");

        assert_eq!(parsed.options.direct_packets_stat, u64::from(u32::MAX) + 1);
        assert_eq!(parsed.options.direct_qlen, u64::from(u32::MAX) + 2);
        assert_eq!(parsed.packets, u64::from(u32::MAX) + 4);
        assert_eq!(parsed.drops, u64::from(u32::MAX) + 5);
        assert_eq!(parsed.backlog, u64::from(u32::MAX) + 8);
    }
}
