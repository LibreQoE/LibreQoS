//! Provides a globally accessible vector of all flows. This is used to store
//! all flows for the purpose of tracking and data-services.

use crate::throughput_tracker::tracking_data::MAX_RETRY_TIMES;

use super::{RttData, flow_analysis::FlowAnalysis, RttBuffer};
use allocative_derive::Allocative;
use fxhash::FxHashMap;
use lqos_sys::flowbee_data::{FlowbeeData, FlowbeeKey};
use lqos_utils::qoo::QoqScores;
use lqos_utils::rtt::RttBucket;
use lqos_utils::units::DownUpOrder;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::ser::SerializeStruct;
use serde::ser::SerializeTuple;
use serde::{Serialize, Serializer};
use smallvec::SmallVec;
use crate::throughput_tracker::flow_data::flow_analysis::FlowbeeEffectiveDirection;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Allocative)]
pub struct AsnId(pub u32);

pub static ALL_FLOWS: Lazy<Mutex<FlowTracker>> = Lazy::new(|| Mutex::new(FlowTracker::default()));

#[derive(Default, Allocative)]
pub struct FlowTracker {
    pub flow_data: FxHashMap<FlowbeeKey, (FlowbeeLocalData, FlowAnalysis)>,
}

#[derive(Clone)]
struct RetryTimesWire {
    idx: usize,
    times: [u64; MAX_RETRY_TIMES],
}

impl Serialize for RetryTimesWire {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut tup = serializer.serialize_tuple(2)?;
        tup.serialize_element(&self.idx)?;
        tup.serialize_element(&self.times.as_slice())?;
        tup.end()
    }
}

#[derive(Debug, Clone, Allocative)]
pub struct FlowbeeLocalDataTcp {
    /// Raw TCP flags
    pub flags: u8,
    /// Recent RTT data for the flow
    pub rtt: RttBuffer,
    /// QoQ scores (0..100) for the flow, derived from RTT/throughput/retransmits.
    pub qoq: QoqScores,
    /// When did the retries happen? In nanoseconds since kernel boot
    #[allocative(skip)]
    pub retry_times_down: SmallVec<[u64; 2]>,
    /// When did the retries happen? In nanoseconds since kernel boot
    #[allocative(skip)]
    pub retry_times_up: SmallVec<[u64; 2]>,
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
    /// TC handle from the `ip_info` match (0 if unshaped).
    pub tc_handle: u32,
    /// CPU mapping from the `ip_info` match (0 if unshaped).
    pub cpu: u32,
    /// Hashed circuit identifier (bit-pattern of `hash_to_i64` stored as `u64`).
    pub circuit_hash: Option<i64>,
    /// Hashed device identifier (bit-pattern of `hash_to_i64` stored as `u64`).
    pub device_hash: Option<i64>,
    /// TCP-only data. Boxed for now; TODO: use a slab/slot type setup for coherence in the future.
    pub tcp_info: Option<Box<FlowbeeLocalDataTcp>>,
}

impl Serialize for FlowbeeLocalData {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Note: Keep this wire format stable (UI compatibility) while we refactor internal storage.
        let mut state = serializer.serialize_struct("FlowbeeLocalData", 13)?;
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
        state.serialize_field("qoq", &self.get_qoq_scores())?;
        let retry_times_down = self.get_retry_times_down_wire();
        let retry_times_up = self.get_retry_times_up_wire();
        state.serialize_field("retry_times_down", &retry_times_down)?;
        state.serialize_field("retry_times_up", &retry_times_up)?;

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
            tc_handle: data.tc_handle,
            cpu: data.cpu,
            circuit_hash: if data.circuit_hash == 0 {
                None
            } else {
                Some(data.circuit_hash as i64)
            },
            device_hash: if data.device_hash == 0 {
                None
            } else {
                Some(data.device_hash as i64)
            },
            tcp_info: if key.ip_protocol == 6 {
                Some(Box::new(FlowbeeLocalDataTcp {
                    flags: data.flags,
                    rtt: RttBuffer::default(),
                    qoq: QoqScores::default(),
                    retry_times_down: SmallVec::new(),
                    retry_times_up: SmallVec::new(),
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
        tcp_info
            .rtt
            .percentile(RttBucket::Current, direction, 50)
            .unwrap_or(RttData::from_nanos(0))
            .as_nanos()
    }

    pub fn get_summary_rtt_as_micros(&self, direction: FlowbeeEffectiveDirection) -> f64 {
        // TODO: This function is due for deprecation, it's in place for
        // compatibility only right now. Kill it with fire!

        let Some(tcp_info) = &self.tcp_info else {
            return 0.0;
        };
        tcp_info
            .rtt
            .percentile(RttBucket::Current, direction, 50)
            .unwrap_or(RttData::from_nanos(0))
            .as_micros()
    }

    pub fn get_summary_rtt_as_millis(&self, direction: FlowbeeEffectiveDirection) -> f64 {
        // TODO: This function is due for deprecation, it's in place for
        // compatibility only right now. Kill it with fire!

        let Some(tcp_info) = &self.tcp_info else {
            return 0.0;
        };
        tcp_info
            .rtt
            .percentile(RttBucket::Current, direction, 50)
            .unwrap_or(RttData::from_nanos(0))
            .as_millis()
    }

    pub fn get_retry_times_down(&self) -> &[u64] {
        let Some(tcp_info) = &self.tcp_info else {
            return &[];
        };
        tcp_info.retry_times_down.as_slice()
    }

    pub fn get_retry_times_up(&self) -> &[u64] {
        let Some(tcp_info) = &self.tcp_info else {
            return &[];
        };
        tcp_info.retry_times_up.as_slice()
    }

    fn retry_times_to_wire(times: &[u64]) -> Option<RetryTimesWire> {
        if times.is_empty() {
            return None;
        }

        let mut buffer = [0u64; MAX_RETRY_TIMES];
        let count = usize::min(times.len(), MAX_RETRY_TIMES);
        buffer[..count].copy_from_slice(&times[..count]);
        Some(RetryTimesWire {
            idx: count,
            times: buffer,
        })
    }

    fn get_retry_times_down_wire(&self) -> Option<RetryTimesWire> {
        let Some(tcp_info) = &self.tcp_info else {
            return None;
        };
        Self::retry_times_to_wire(tcp_info.retry_times_down.as_slice())
    }

    fn get_retry_times_up_wire(&self) -> Option<RetryTimesWire> {
        let Some(tcp_info) = &self.tcp_info else {
            return None;
        };
        Self::retry_times_to_wire(tcp_info.retry_times_up.as_slice())
    }

    pub fn get_rtt_array(&self) -> [RttData; 2] {
        let Some(tcp_info) = &self.tcp_info else {
            return [RttData::from_nanos(0); 2];
        };
        [
            tcp_info
                .rtt
                .percentile(
                    RttBucket::Current,
                    FlowbeeEffectiveDirection::Download,
                    50,
                )
                .unwrap_or(RttData::from_nanos(0)),
            tcp_info
                .rtt
                .percentile(RttBucket::Current, FlowbeeEffectiveDirection::Upload, 50)
                .unwrap_or(RttData::from_nanos(0)),
        ]
    }

    pub fn get_qoq_scores(&self) -> QoqScores {
        let Some(tcp_info) = &self.tcp_info else {
            return QoqScores::default();
        };
        tcp_info.qoq
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
        tcp_info
            .rtt
            .percentile(RttBucket::Current, direction, 50)
            .unwrap_or(RttData::from_nanos(0))
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

    pub fn set_rtt_buffer(&mut self, rtt: RttBuffer) {
        let Some(tcp_info) = &mut self.tcp_info else {
            return;
        };
        tcp_info.rtt.merge_fresh_from(rtt);
    }

    pub fn set_qoq_scores(&mut self, scores: QoqScores) {
        let Some(tcp_info) = &mut self.tcp_info else {
            return;
        };
        tcp_info.qoq = scores;
    }

    pub fn record_tcp_retry_time(&mut self, direction: FlowbeeEffectiveDirection, timestamp_nanos: u64) {
        let Some(tcp_info) = &mut self.tcp_info else {
            return;
        };

        let target = match direction {
            FlowbeeEffectiveDirection::Download => &mut tcp_info.retry_times_down,
            FlowbeeEffectiveDirection::Upload => &mut tcp_info.retry_times_up,
        };

        // Keep the most recent `MAX_RETRY_TIMES` entries.
        if target.len() >= MAX_RETRY_TIMES {
            // Not a hot path, and MAX is small enough that shifting is OK.
            target.remove(0);
        }
        target.push(timestamp_nanos);
    }
}
