use lqos_utils::XdpIpAddress;
use zerocopy::FromBytes;

/// Representation of the XDP map from map_traffic
#[repr(C)]
#[derive(Debug, Clone, Default, FromBytes)]
pub struct HostCounter {
    /// Download bytes counter (keeps incrementing)
    pub download_bytes: u64,

    /// Upload bytes counter (keeps incrementing)
    pub upload_bytes: u64,

    /// Download packets counter (keeps incrementing)
    pub download_packets: u64,

    /// Upload packets counter (keeps incrementing)
    pub upload_packets: u64,

    /// TCP packets downloaded
    pub tcp_download_packets: u64,

    /// TCP packets uploaded
    pub tcp_upload_packets: u64,

    /// UDP packets downloaded
    pub udp_download_packets: u64,

    /// UDP packets uploaded
    pub udp_upload_packets: u64,

    /// ICMP packets downloaded
    pub icmp_download_packets: u64,

    /// ICMP packets uploaded
    pub icmp_upload_packets: u64,

    /// Mapped TC handle, 0 if there isn't one.
    pub tc_handle: u32,

    /// Hashed circuit identifier (from ShapedDevices.csv), 0 if unknown/unshaped.
    pub circuit_id: u64,

    /// Hashed device identifier (from ShapedDevices.csv), 0 if unknown/unshaped.
    pub device_id: u64,

    /// Time last seen, in nanoseconds since kernel boot
    pub last_seen: u64,
}

#[cfg(test)]
mod test {
    use super::HostCounter;

    #[test]
    fn host_counter_size() {
        assert_eq!(std::mem::size_of::<HostCounter>(), 112);
    }
}

/// Iterates through all throughput entries, and sends them in turn to `callback`.
/// This elides the need to clone or copy data.
pub fn throughput_for_each(callback: &mut dyn FnMut(&XdpIpAddress, &[HostCounter])) {
    unsafe {
        crate::bpf_iterator::iterate_throughput(callback);
    }
}
