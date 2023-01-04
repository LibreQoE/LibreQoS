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

use anyhow::{Result, Error};
use lqos_bus::TcHandle;
use serde::Serialize;
use serde_json::Value;

#[derive(Default, Clone, Debug, Serialize)]
pub(crate) struct TcCake {
    pub(crate) handle: TcHandle,
    pub(crate) parent: TcHandle,
    options: TcCakeOptions,
    bytes: u64,
    packets: u64,
    overlimits: u64,
    requeues: u64,
    backlog: u64,
    qlen: u64,
    memory_used: u64,
    memory_limit: u64,
    capacity_estimate: u64,
    min_network_size: u64,
    max_network_size: u64,
    min_adj_size: u64,
    max_adj_size: u64,
    avg_hdr_offset: u64,
    tins: Vec<TcCakeTin>,
    drops: u64,
 }

 #[derive(Default, Clone, Debug, Serialize)]
  struct TcCakeOptions {
    bandwidth: String,
    diffserv: String,
    flowmode: String,
    nat: bool,
    wash: bool,
    ingress: bool,
    ack_filter: String,
    split_gso: bool,
    rtt: u64,
    raw: bool,
    overhead: u64,
    fwmark: String,
 }

 #[derive(Default, Clone, Debug, Serialize)]
 struct TcCakeTin {
    threshold_rate: u64,
    sent_bytes: u64,
    backlog_bytes: u64,
    target_us: u64,
    interval_us: u64,
    peak_delay_us: u64,
    avg_delay_us: u64,
    base_delay_us: u64,
    sent_packets: u64,
    way_indirect_hits: u64,
    way_misses: u64,
    way_collisions: u64,
    drops: u64,
    ecn_marks: u64,
    ack_drops: u64,
    sparse_flows: u64,
    bulk_flows: u64,
    unresponsive_flows: u64,
    max_pkt_len: u64,
    flow_quantum: u64,
 }

 impl TcCake {
    pub(crate) fn from_json(map: &serde_json::Map<std::string::String, Value>) -> Result<Self> {
        let mut result = Self::default();
        for (key, value) in map.iter() {
            match key.as_str() {
                "handle" => result.handle = TcHandle::from_string(value.as_str().unwrap())?,
                "parent" => result.parent = TcHandle::from_string(value.as_str().unwrap())?,
                "bytes" => result.bytes = value.as_u64().unwrap(),
                "packets" => result.packets = value.as_u64().unwrap(),
                "overlimits" => result.overlimits = value.as_u64().unwrap(),
                "requeues" => result.requeues = value.as_u64().unwrap(),
                "backlog" => result.backlog = value.as_u64().unwrap(),
                "qlen" => result.qlen = value.as_u64().unwrap(),
                "memory_used" => result.memory_used = value.as_u64().unwrap(),
                "memory_limit" => result.memory_limit = value.as_u64().unwrap(),
                "capacity_estimate" => result.capacity_estimate = value.as_u64().unwrap(),
                "min_network_size" => result.min_network_size = value.as_u64().unwrap(),
                "max_network_size" => result.max_network_size = value.as_u64().unwrap(),
                "min_adj_size" => result.min_adj_size = value.as_u64().unwrap(),
                "max_adj_size" => result.max_adj_size = value.as_u64().unwrap(),
                "avg_hdr_offset" => result.avg_hdr_offset = value.as_u64().unwrap(),
                "drops" => result.drops = value.as_u64().unwrap(),
                "options" => result.options = TcCakeOptions::from_json(value)?,
                "tins" => {
                    match value {
                        Value::Array(array) => {
                            for value in array.iter() {
                                result.tins.push(TcCakeTin::from_json(value)?);
                            }
                        }
                        _ => {}
                    }
                }
                "kind" => {},
               _ => {
                    log::error!("Unknown entry in Tc-cake: {key}");
                }
            }
        }
        Ok(result)
    }
}

impl TcCakeOptions {
    fn from_json(value: &Value) -> Result<Self> {
        match value {
            Value::Object(map) => {
                let mut result = Self::default();
                for (key, value) in map.iter() {
                    match key.as_str() {
                        "bandwidth" => result.bandwidth = value.as_str().unwrap().to_string(),
                        "diffserv" => result.diffserv = value.as_str().unwrap().to_string(),
                        "flowmode" => result.flowmode = value.as_str().unwrap().to_string(),
                        "nat" => result.nat = value.as_bool().unwrap(),
                        "wash" => result.wash = value.as_bool().unwrap(),
                        "ingress" => result.ingress = value.as_bool().unwrap(),
                        "ack-filter" => result.ack_filter = value.as_str().unwrap().to_string(),
                        "split_gso" => result.split_gso = value.as_bool().unwrap(),
                        "rtt" => result.rtt = value.as_u64().unwrap(),
                        "raw" => result.raw = value.as_bool().unwrap(),
                        "overhead" => result.overhead = value.as_u64().unwrap(),
                        "fwmark" => result.fwmark = value.as_str().unwrap().to_string(),
                        _ => {
                            log::error!("Unknown entry in Tc-cake-options: {key}");
                        }
                    }
                }
                Ok(result)
            }
            _ => Err(Error::msg("Unable to parse cake options")),
        }        
    }
}

impl TcCakeTin {
    fn from_json(value: &Value) -> Result<Self> {
        match value {
            Value::Object(map) => {
                let mut result = Self::default();
                for (key, value) in map.iter() {
                    match key.as_str() {
                        "threshold_rate" => result.threshold_rate = value.as_u64().unwrap(),
                        "sent_bytes" => result.sent_bytes = value.as_u64().unwrap(),
                        "backlog_bytes" => result.backlog_bytes = value.as_u64().unwrap(),
                        "target_us" => result.target_us = value.as_u64().unwrap(),
                        "interval_us" => result.interval_us = value.as_u64().unwrap(),
                        "peak_delay_us" => result.peak_delay_us = value.as_u64().unwrap(),
                        "avg_delay_us" => result.avg_delay_us = value.as_u64().unwrap(),
                        "base_delay_us" => result.base_delay_us = value.as_u64().unwrap(),
                        "sent_packets" => result.sent_packets = value.as_u64().unwrap(),
                        "way_indirect_hits" => result.way_indirect_hits = value.as_u64().unwrap(),
                        "way_misses" => result.way_misses = value.as_u64().unwrap(),
                        "way_collisions" => result.way_collisions = value.as_u64().unwrap(),
                        "drops" => result.drops = value.as_u64().unwrap(),
                        "ecn_mark" => result.ecn_marks = value.as_u64().unwrap(),
                        "ack_drops" => result.ack_drops = value.as_u64().unwrap(),
                        "sparse_flows" => result.sparse_flows = value.as_u64().unwrap(),
                        "bulk_flows" => result.bulk_flows = value.as_u64().unwrap(),
                        "unresponsive_flows" => result.unresponsive_flows = value.as_u64().unwrap(),
                        "max_pkt_len" => result.max_pkt_len = value.as_u64().unwrap(),
                        "flow_quantum" => result.flow_quantum = value.as_u64().unwrap(),
                        _ => {
                            log::error!("Unknown entry in Tc-cake-tin: {key}");
                        }
                    }
                }
                Ok(result)
            }
            _ => Err(Error::msg("Unable to parse cake tin options")),
        }        
    }
}