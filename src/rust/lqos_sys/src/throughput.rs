use lqos_utils::XdpIpAddress;
use zerocopy::FromBytes;

/// Representation of the XDP map from map_traffic
#[repr(C)]
#[derive(Debug, Clone, Default, FromBytes)]
pub struct HostCounter {
    /// Enqueued download bytes counter (keeps incrementing)
    pub enqueue_download_bytes: u64,

    /// Enqueued upload bytes counter (keeps incrementing)
    pub enqueue_upload_bytes: u64,

    /// Enqueued download packets counter (keeps incrementing)
    pub enqueue_download_packets: u64,

    /// Enqueued upload packets counter (keeps incrementing)
    pub enqueue_upload_packets: u64,

    /// Enqueued TCP packets downloaded
    pub enqueue_tcp_download_packets: u64,

    /// Enqueued TCP packets uploaded
    pub enqueue_tcp_upload_packets: u64,

    /// Enqueued UDP packets downloaded
    pub enqueue_udp_download_packets: u64,

    /// Enqueued UDP packets uploaded
    pub enqueue_udp_upload_packets: u64,

    /// Enqueued ICMP packets downloaded
    pub enqueue_icmp_download_packets: u64,

    /// Enqueued ICMP packets uploaded
    pub enqueue_icmp_upload_packets: u64,

    /// Mapped TC handle, 0 if there isn't one.
    pub tc_handle: u32,

    /// Hashed circuit identifier (from ShapedDevices.csv), 0 if unknown/unshaped.
    pub circuit_id: u64,

    /// Hashed device identifier (from ShapedDevices.csv), 0 if unknown/unshaped.
    pub device_id: u64,

    /// Time last seen, in nanoseconds since kernel boot
    pub last_seen: u64,
}

/// Iterates through all throughput entries, and sends them in turn to `callback`.
/// This elides the need to clone or copy data.
pub fn throughput_for_each(callback: &mut dyn FnMut(&XdpIpAddress, &[HostCounter])) {
    unsafe {
        crate::bpf_iterator::iterate_throughput(callback);
    }
}

#[cfg(test)]
mod test {
    use super::HostCounter;

    #[test]
    fn host_counter_size() {
        assert_eq!(std::mem::size_of::<HostCounter>(), 112);
    }
}
