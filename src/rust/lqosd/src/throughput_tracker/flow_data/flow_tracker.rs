//! Provides a globally accessible vector of all flows. This is used to store
//! all flows for the purpose of tracking and data-services.

use crate::throughput_tracker::tracking_data::MAX_RETRY_TIMES;

use super::{RttData, flow_analysis::FlowAnalysis};
use allocative_derive::Allocative;
use fxhash::FxHashMap;
use lqos_sys::flowbee_data::{FlowbeeData, FlowbeeKey};
use lqos_utils::units::DownUpOrder;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};
use crate::throughput_tracker::flow_data::flow_analysis::FlowbeeEffectiveDirection;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Allocative)]
pub struct AsnId(pub u32);

pub static ALL_FLOWS: Lazy<Mutex<FlowTracker>> = Lazy::new(|| Mutex::new(FlowTracker::default()));

#[derive(Default, Allocative)]
pub struct FlowTracker {
    pub flow_data: FxHashMap<FlowbeeKey, (FlowbeeLocalData, FlowAnalysis)>,
}

#[derive(Debug, Clone, Serialize, Allocative)]
pub struct FlowbeeLocalDataTcp {
    /// Raw TCP flags
    pub flags: u8,
    /// Recent RTT median
    pub rtt: [RttData; 2],
    /// When did the retries happen? In nanoseconds since kernel boot
    pub retry_times_down: Option<(usize, [u64; MAX_RETRY_TIMES])>,
    /// When did the retries happen? In nanoseconds since kernel boot
    pub retry_times_up: Option<(usize, [u64; MAX_RETRY_TIMES])>,
}

/// Condensed representation of the FlowbeeData type. This contains
/// only the information we want to keep locally for analysis purposes,
/// adds RTT data, and uses Rust-friendly typing.
#[derive(Debug, Clone, Allocative)]
pub struct FlowbeeLocalData {
    /// Time (nanos) when the connection was established
    pub start_time: u64,
    /// Time (nanos) when the connection was last seen
    pub last_seen: u64,
    /// Bytes transmitted
    pub bytes_sent: DownUpOrder<u64>,
    /// Packets transmitted
    pub packets_sent: DownUpOrder<u64>,
    /// Rate estimate
    pub rate_estimate_bps: DownUpOrder<u32>,
    /// TCP Retransmission count (also counts duplicates)
    pub tcp_retransmits: DownUpOrder<u16>,
    /// Has the connection ended?
    /// 0 = Alive, 1 = FIN, 2 = RST
    pub end_status: u8,
    /// Raw IP TOS
    pub tos: u8,
    /// TCP-only data. Boxed for now; TODO: use a slab/slot type setup for coherence in the future.
    pub tcp_info: Option<Box<FlowbeeLocalDataTcp>>,
}

impl Serialize for FlowbeeLocalData {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Note: Keep this wire format stable (UI compatibility) while we refactor internal storage.
        let mut state = serializer.serialize_struct("FlowbeeLocalData", 12)?;
        state.serialize_field("start_time", &self.start_time)?;
        state.serialize_field("last_seen", &self.last_seen)?;
        state.serialize_field("bytes_sent", &self.bytes_sent)?;
        state.serialize_field("packets_sent", &self.packets_sent)?;
        state.serialize_field("rate_estimate_bps", &self.rate_estimate_bps)?;
        state.serialize_field("tcp_retransmits", &self.tcp_retransmits)?;
        state.serialize_field("end_status", &self.end_status)?;
        state.serialize_field("tos", &self.tos)?;

        // TCP-only fields (default to zero/None if this isn't a TCP flow).
        state.serialize_field("flags", &self.get_flags())?;
        state.serialize_field("rtt", &self.get_rtt_array())?;
        state.serialize_field("retry_times_down", self.get_retry_times_down())?;
        state.serialize_field("retry_times_up", self.get_retry_times_up())?;

        state.end()
    }
}

impl FlowbeeLocalData {
    pub fn from_flow(data: &FlowbeeData, key: &FlowbeeKey) -> Self {
        Self {
            start_time: data.start_time,
            last_seen: data.last_seen,
            bytes_sent: data.bytes_sent,
            packets_sent: data.packets_sent,
            rate_estimate_bps: data.rate_estimate_bps,
            tcp_retransmits: data.tcp_retransmits,
            end_status: data.end_status,
            tos: data.tos,
            tcp_info: if key.ip_protocol == 6 {
                Some(Box::new(FlowbeeLocalDataTcp {
                    flags: data.flags,
                    rtt: [RttData::from_nanos(0); 2],
                    retry_times_down: None,
                    retry_times_up: None,
                }))
            } else {
                None
            },
        }
    }

    pub fn get_summary_rtt_as_nanos(&self, direction: FlowbeeEffectiveDirection) -> u64 {
        // TODO: This function is due for deprecation, it's in place for
        // compatibility only right now. Kill it with fire!

        let Some(tcp_info) = &self.tcp_info else {
            return 0;
        };
        tcp_info.rtt[direction as usize].as_nanos()
    }

    pub fn get_summary_rtt_as_micros(&self, direction: FlowbeeEffectiveDirection) -> f64 {
        // TODO: This function is due for deprecation, it's in place for
        // compatibility only right now. Kill it with fire!

        let Some(tcp_info) = &self.tcp_info else {
            return 0.0;
        };
        tcp_info.rtt[direction as usize].as_micros()
    }

    pub fn get_summary_rtt_as_millis(&self, direction: FlowbeeEffectiveDirection) -> f64 {
        // TODO: This function is due for deprecation, it's in place for
        // compatibility only right now. Kill it with fire!

        let Some(tcp_info) = &self.tcp_info else {
            return 0.0;
        };
        tcp_info.rtt[direction as usize].as_millis()
    }

    pub fn get_retry_times_down(&self) -> &Option<(usize, [u64; MAX_RETRY_TIMES])> {
        let Some(tcp_info) = &self.tcp_info else {
            return &None;
        };
        &tcp_info.retry_times_down
    }

    pub fn get_retry_times_up(&self) -> &Option<(usize, [u64; MAX_RETRY_TIMES])> {
        let Some(tcp_info) = &self.tcp_info else {
            return &None;
        };
        &tcp_info.retry_times_up
    }

    pub fn get_rtt_array(&self) -> [RttData; 2] {
        let Some(tcp_info) = &self.tcp_info else {
            return [RttData::from_nanos(0); 2];
        };
        tcp_info.rtt.clone()
    }

    pub fn get_flags(&self) -> u8 {
        let Some(tcp_info) = &self.tcp_info else {
            return 0;
        };
        tcp_info.flags
    }

    pub fn get_rtt(&self, direction: FlowbeeEffectiveDirection) -> RttData {
        let Some(tcp_info) = &self.tcp_info else {
            return RttData::from_nanos(0);
        };
        tcp_info.rtt[direction as usize].clone()
    }

    pub fn set_last_seen(&mut self, last_seen: u64) {
        self.last_seen = last_seen;
    }

    pub fn set_bytes_sent(&mut self, bytes_sent: DownUpOrder<u64>) {
        self.bytes_sent = bytes_sent;
    }

    pub fn set_packets_sent(&mut self, packets_sent: DownUpOrder<u64>) {
        self.packets_sent = packets_sent;
    }

    pub fn set_rate_estimate_bps(&mut self, rate_estimate_bps: DownUpOrder<u32>) {
        self.rate_estimate_bps = rate_estimate_bps;
    }

    pub fn set_tcp_retransmits(&mut self, tcp_retransmits: DownUpOrder<u16>) {
        self.tcp_retransmits = tcp_retransmits;
    }

    pub fn set_end_status(&mut self, end_status: u8) {
        self.end_status = end_status;
    }

    pub fn set_tos(&mut self, tos: u8) {
        self.tos = tos;
    }

    pub fn set_flags(&mut self, flags: u8) {
        let Some(tcp_info) = &mut self.tcp_info else {
            return;
        };
        tcp_info.flags = flags;
    }

    pub fn set_rtt_if_non_zero(&mut self, direction: FlowbeeEffectiveDirection, rtt: RttData) {
        if rtt.as_nanos() == 0 {
            return;
        }
        let Some(tcp_info) = &mut self.tcp_info else {
            return;
        };
        tcp_info.rtt[direction as usize] = rtt;
    }

    pub fn record_tcp_retry_time(&mut self, direction: FlowbeeEffectiveDirection, timestamp_nanos: u64) {
        let Some(tcp_info) = &mut self.tcp_info else {
            return;
        };

        let target = match direction {
            FlowbeeEffectiveDirection::Download => &mut tcp_info.retry_times_down,
            FlowbeeEffectiveDirection::Upload => &mut tcp_info.retry_times_up,
        };

        let (idx, times) = target.get_or_insert((0, [0; MAX_RETRY_TIMES]));
        times[*idx] = timestamp_nanos;
        *idx = (*idx + 1) % MAX_RETRY_TIMES;
    }
}
