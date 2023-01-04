use anyhow::Result;

use crate::bpf_map::BpfMap;

/// Entry from the XDP rtt_tracker map.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RttTrackingEntry {
    /// Array containing TCP round-trip times. Convert to an `f32` and divide by `100.0` for actual numbers.
    pub rtt: [u32; 60],
    
    /// Used internally by the XDP program to store the current position in the storage array. Do not modify.
    next_entry: u32,

    /// Used internally by the XDP program to determine when it is time to recycle and reuse a record. Do not modify.
    recycle_time: u64,

    /// Flag indicating that an entry has been updated recently (last 30 seconds by default).
    pub has_fresh_data: u32,
}

impl Default for RttTrackingEntry {
    fn default() -> Self {
        Self {
            rtt: [0; 60],
            next_entry: 0,
            recycle_time: 0,
            has_fresh_data: 0,
        }
    }
}

/// Queries the active XDP/TC programs for TCP round-trip time tracking
/// data (from the `rtt_tracker` pinned eBPF map).
/// 
/// Only IP addresses facing the ISP Network side are tracked.
/// 
/// ## Returns
/// 
/// A vector containing:
/// * `[u8; 16]` - a byte representation of the encoded IP address. See `XdpIpAddress` for details.
/// * An `RttTrackingEntry` structure containing the current RTT results for the IP address.
pub fn get_tcp_round_trip_times() -> Result<Vec<([u8; 16], RttTrackingEntry)>> {
    let rtt_tracker = BpfMap::<[u8; 16], RttTrackingEntry>::from_path("/sys/fs/bpf/rtt_tracker")?;
    let rtt_data = rtt_tracker.dump_vec();
    Ok(rtt_data)
}
