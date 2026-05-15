use crate::timeline::store_on_timeline;
use lqos_utils::XdpIpAddress;
use std::{ffi::c_void, slice};
use tracing::warn;
use zerocopy::FromBytes;

/// This constant MUST exactly match PACKET_OCTET_STATE in heimdall.h
pub(crate) const PACKET_OCTET_SIZE: usize = 128;

/// A representation of the eBPF `heimdall_event` type.
/// This is the type that is sent from the eBPF program to userspace.
/// It is a representation of the `heimdall_event` type in heimdall.h
#[derive(FromBytes, Debug, Clone, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct HeimdallEvent {
    /// Timestamp of the event, in nanoseconds since boot time.
    pub timestamp: u64,
    /// Source IP address
    pub src: XdpIpAddress,
    /// Destination IP address
    pub dst: XdpIpAddress,
    /// Source port number, or ICMP type.
    pub src_port: u16,
    /// Destination port number.
    pub dst_port: u16,
    /// IP protocol number
    pub ip_protocol: u8,
    /// IP header TOS value
    pub tos: u8,
    /// Total size of the packet, in bytes
    pub size: u32,
    /// TCP flags
    pub tcp_flags: u8,
    /// TCP window size
    pub tcp_window: u16,
    /// TCP sequence number
    pub tcp_tsval: u32,
    /// TCP acknowledgement number
    pub tcp_tsecr: u32,
    /// Raw packet data
    pub packet_data: [u8; PACKET_OCTET_SIZE],
}

impl Default for HeimdallEvent {
    fn default() -> Self {
        Self {
            timestamp: 0,
            src: XdpIpAddress::default(),
            dst: XdpIpAddress::default(),
            src_port: 0,
            dst_port: 0,
            ip_protocol: 0,
            tos: 0,
            size: 0,
            tcp_flags: 0,
            tcp_window: 0,
            tcp_tsval: 0,
            tcp_tsecr: 0,
            packet_data: [0; PACKET_OCTET_SIZE],
        }
    }
}

impl HeimdallEvent {
    pub(crate) fn captured_len(&self) -> usize {
        (self.size as usize).min(PACKET_OCTET_SIZE)
    }

    pub(crate) fn packet_bytes(&self) -> &[u8] {
        &self.packet_data[..self.captured_len()]
    }

    fn clamp_size_to_capture_buffer(&mut self) {
        self.size = self.captured_len() as u32;
    }
}

/*
Snippet for tcp_flags decoding
if (hdr->fin) flags |= 1;
if (hdr->syn) flags |= 2;
if (hdr->rst) flags |= 4;
if (hdr->psh) flags |= 8;
if (hdr->ack) flags |= 16;
if (hdr->urg) flags |= 32;
if (hdr->ece) flags |= 64;
if (hdr->cwr) flags |= 128;
 */

/// Callback for the Heimdall Perf map system. Called whenever Heimdall has
/// events for the system to read.
///
/// # Safety
///
/// This function is inherently unsafe, because it interfaces directly with
/// C and the Linux-kernel eBPF system.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn heimdall_handle_events(
    _ctx: *mut c_void,
    data: *mut c_void,
    data_size: usize,
) -> i32 {
    const EVENT_SIZE: usize = std::mem::size_of::<HeimdallEvent>();
    if data_size < EVENT_SIZE {
        warn!("Warning: incoming data too small in Heimdall buffer");
        return 0;
    }

    //COLLECTED_EVENTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let data_u8 = data as *const u8;
    let data_slice: &[u8] = unsafe { slice::from_raw_parts(data_u8, EVENT_SIZE) };

    if let Ok(mut incoming) = HeimdallEvent::read_from_bytes(data_slice) {
        incoming.clamp_size_to_capture_buffer();
        store_on_timeline(incoming);
    } else {
        println!("Failed to decode");
    }

    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captured_len_clamps_to_packet_buffer() {
        let event = HeimdallEvent {
            size: PACKET_OCTET_SIZE as u32 + 64,
            ..Default::default()
        };

        assert_eq!(event.captured_len(), PACKET_OCTET_SIZE);
    }

    #[test]
    fn packet_bytes_honors_stated_capture_size() {
        let mut event = HeimdallEvent {
            size: 4,
            ..Default::default()
        };
        event.packet_data[..6].copy_from_slice(&[1, 2, 3, 4, 5, 6]);

        assert_eq!(event.packet_bytes(), &[1, 2, 3, 4]);
    }
}
