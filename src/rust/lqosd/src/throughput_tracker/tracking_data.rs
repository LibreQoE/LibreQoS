use std::collections::HashMap;
use lqos_bus::TcHandle;
use lqos_sys::{XdpIpAddress, HostCounter, RttTrackingEntry};
use anyhow::Result;
use super::{throughput_entry::ThroughputEntry, RETIRE_AFTER_SECONDS};

pub struct ThroughputTracker {
    pub(crate) cycle: u64,
    pub(crate) raw_data: HashMap<XdpIpAddress, ThroughputEntry>,
    pub(crate) bytes_per_second: (u64, u64),
    pub(crate) packets_per_second: (u64, u64),
    pub(crate) shaped_bytes_per_second: (u64, u64),
}

impl ThroughputTracker {
    pub(crate) fn new() -> Self {
        // The capacity should match that found in
        // maximums.h (MAX_TRACKED_IPS), so we grab it
        // from there via the C API.
        Self {
            cycle: RETIRE_AFTER_SECONDS,
            raw_data: HashMap::with_capacity(lqos_sys::max_tracked_ips()),
            bytes_per_second: (0, 0),
            packets_per_second: (0, 0),
            shaped_bytes_per_second: (0, 0),
        }
    }

    pub(crate) fn tick(&mut self, value_dump: &[(XdpIpAddress, Vec<HostCounter>)], rtt: Result<Vec<([u8; 16], RttTrackingEntry)>>) -> Result<()> {
        // Copy previous byte/packet numbers and reset RTT data
        self.raw_data.iter_mut().for_each(|(_k, v)| {
            if v.first_cycle < self.cycle {
                v.bytes_per_second.0 = v.bytes.0 - v.prev_bytes.0;
                v.bytes_per_second.1 = v.bytes.1 - v.prev_bytes.1;
                v.packets_per_second.0 = v.packets.0 - v.prev_packets.0;
                v.packets_per_second.1 = v.packets.1 - v.prev_packets.1;
                v.prev_bytes = v.bytes;
                v.prev_packets = v.packets;
            }
            // Roll out stale RTT data
            if self.cycle > RETIRE_AFTER_SECONDS && v.last_fresh_rtt_data_cycle < self.cycle - RETIRE_AFTER_SECONDS {
                v.recent_rtt_data = [0; 60];
            }
        });

        value_dump.iter().for_each(|(xdp_ip, counts)| {
            if let Some(entry) = self.raw_data.get_mut(xdp_ip) {
                entry.bytes = (0, 0);
                entry.packets = (0, 0);
                for c in counts {
                    entry.bytes.0 += c.download_bytes;
                    entry.bytes.1 += c.upload_bytes;
                    entry.packets.0 += c.download_packets;
                    entry.packets.1 += c.upload_packets;
                    if c.tc_handle != 0 {
                        entry.tc_handle = TcHandle::from_u32(c.tc_handle);
                    }
                }
                if entry.packets != entry.prev_packets {
                    entry.most_recent_cycle = self.cycle;
                }
            } else {
                let mut entry = ThroughputEntry {
                    first_cycle: self.cycle,
                    most_recent_cycle: 0,
                    bytes: (0, 0),
                    packets: (0, 0),
                    prev_bytes: (0, 0),
                    prev_packets: (0, 0),
                    bytes_per_second: (0, 0),
                    packets_per_second: (0, 0),
                    tc_handle: TcHandle::zero(),
                    recent_rtt_data: [0; 60],
                    last_fresh_rtt_data_cycle: 0,
                };
                for c in counts {
                    entry.bytes.0 += c.download_bytes;
                    entry.bytes.1 += c.upload_bytes;
                    entry.packets.0 += c.download_packets;
                    entry.packets.1 += c.upload_packets;
                    if c.tc_handle != 0 {
                        entry.tc_handle = TcHandle::from_u32(c.tc_handle);
                    }
                }
                self.raw_data.insert(*xdp_ip, entry);
            }
        });

        // Apply RTT data
        if let Ok(rtt_dump) = rtt {
            for (raw_ip, rtt) in rtt_dump {
                if rtt.has_fresh_data != 0 {
                    let ip = XdpIpAddress(raw_ip);
                    if let Some(tracker) = self.raw_data.get_mut(&ip) {
                        tracker.recent_rtt_data = rtt.rtt;
                        tracker.last_fresh_rtt_data_cycle = self.cycle;
                    }
                }
            }
        }

        // Update totals
        self.bytes_per_second = (0, 0);
        self.packets_per_second = (0, 0);
        self.shaped_bytes_per_second = (0, 0);
        self.raw_data
            .iter()
            .map(|(_k, v)| {
                (
                    v.bytes.0 - v.prev_bytes.0,
                    v.bytes.1 - v.prev_bytes.1,
                    v.packets.0 - v.prev_packets.0,
                    v.packets.1 - v.prev_packets.1,
                    v.tc_handle.as_u32() > 0
                )
            })
            .for_each(|(bytes_down, bytes_up, packets_down, packets_up, shaped)| {
                self.bytes_per_second.0 += bytes_down;
                self.bytes_per_second.1 += bytes_up;
                self.packets_per_second.0 += packets_down;
                self.packets_per_second.1 += packets_up;
                if shaped {
                    self.shaped_bytes_per_second.0 += bytes_down;
                    self.shaped_bytes_per_second.1 += bytes_up;
                }
            });

        // Onto the next cycle
        self.cycle += 1;
        Ok(())
    }

    pub(crate) fn bits_per_second(&self) -> (u64, u64) {
        (self.bytes_per_second.0 * 8, self.bytes_per_second.1 * 8)
    }

    pub(crate) fn shaped_bits_per_second(&self) -> (u64, u64) {
        (self.shaped_bytes_per_second.0 * 8, self.shaped_bytes_per_second.1 * 8)
    }

    pub(crate) fn packets_per_second(&self) -> (u64, u64) {
        self.packets_per_second
    }

    #[allow(dead_code)]
    pub(crate) fn dump(&self) {
        for (k, v) in self.raw_data.iter() {
            let ip = k.as_ip();
            log::info!("{:<34}{:?}", ip, v.tc_handle);
        }
    }
}