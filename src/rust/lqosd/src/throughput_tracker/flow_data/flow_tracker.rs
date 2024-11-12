//! Provides a globally accessible vector of all flows. This is used to store
//! all flows for the purpose of tracking and data-services.

use super::{flow_analysis::FlowAnalysis};
use lqos_sys::flowbee_data::{FlowbeeData, FlowbeeKey};
use once_cell::sync::Lazy;
use std::sync::Mutex;
use fxhash::FxHashMap;
use serde::Serialize;
use lqos_bus::{FlowbeeLocalData, RttData};
use lqos_utils::units::DownUpOrder;

pub static ALL_FLOWS: Lazy<Mutex<FxHashMap<FlowbeeKey, (FlowbeeLocalData, FlowAnalysis)>>> =
    Lazy::new(|| Mutex::new(FxHashMap::default()));

pub fn flowbee_local_from_data(data: &FlowbeeData) -> FlowbeeLocalData {
    FlowbeeLocalData {
        start_time: data.start_time,
        last_seen: data.last_seen,
        bytes_sent: data.bytes_sent,
        packets_sent: data.packets_sent,
        rate_estimate_bps: data.rate_estimate_bps,
        tcp_retransmits: data.tcp_retransmits,
        end_status: data.end_status,
        tos: data.tos,
        flags: data.flags,
        rtt: [RttData::from_nanos(0); 2],
        throughput_buffer: vec![ data.bytes_sent ],
        retry_times_down: Vec::new(),
        retry_times_up: Vec::new(),
    }
}