use crate::perf_interface::{HeimdallEvent, PACKET_OCTET_SIZE};
use std::time::Duration;
use zerocopy::{Immutable, IntoBytes};

#[derive(IntoBytes, Immutable)]
#[repr(C)]
pub(crate) struct PcapFileHeader {
    magic: u32,
    version_major: u16,
    version_minor: u16,
    thiszone: i32,
    sigfigs: u32,
    snaplen: u32,
    link_type: u32,
}

impl PcapFileHeader {
    pub(crate) fn new() -> Self {
        Self {
            magic: 0xa1b2c3d4,
            version_major: 2,
            version_minor: 4,
            thiszone: 0,
            sigfigs: 0,
            snaplen: PACKET_OCTET_SIZE as u32,
            link_type: 1,
        }
    }
}

#[derive(IntoBytes, Immutable)]
#[repr(C)]
pub(crate) struct PcapPacketHeader {
    ts_sec: u32,
    ts_usec: u32,
    inc_len: u32,  // Octets included
    orig_len: u32, // Length the packet used to be
}

impl PcapPacketHeader {
    pub(crate) fn from_heimdall(event: &HeimdallEvent) -> Self {
        let timestamp_nanos = Duration::from_nanos(event.timestamp);
        let captured_len = event.captured_len() as u32;
        Self {
            ts_sec: timestamp_nanos.as_secs() as u32,
            ts_usec: timestamp_nanos.subsec_micros(),
            inc_len: captured_len,
            orig_len: captured_len,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_header_uses_captured_len() {
        let event = HeimdallEvent {
            size: PACKET_OCTET_SIZE as u32 + 1,
            ..Default::default()
        };

        let header = PcapPacketHeader::from_heimdall(&event);

        assert_eq!(header.inc_len, PACKET_OCTET_SIZE as u32);
        assert_eq!(header.orig_len, PACKET_OCTET_SIZE as u32);
    }
}
