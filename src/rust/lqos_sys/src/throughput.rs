use lqos_utils::XdpIpAddress;
use std::ffi::{CString, c_void};
use zerocopy::FromBytes;

/// Representation of the XDP map from map_traffic
#[repr(C)]
#[derive(Debug, Clone, Default, FromBytes)]
pub struct HostCounter {
    /// Download bytes counter (keeps incrementing)
    pub download_bytes: u64,

    /// Upload bytes counter (keeps incrementing)
    pub upload_bytes: u64,

    /// Actually transmitted download bytes counter (keeps incrementing)
    pub actual_download_bytes: u64,

    /// Actually transmitted upload bytes counter (keeps incrementing)
    pub actual_upload_bytes: u64,

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

/// Per-CPU host counter map pressure reported by the eBPF datapath.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, FromBytes)]
pub struct TrafficMapPressure {
    /// Failed attempts to insert a host counter entry.
    pub insert_failures: u64,
    /// Most recent failure time in nanoseconds since kernel boot.
    pub last_failure_ns: u64,
}

/// Iterates through all throughput entries, and sends them in turn to `callback`.
/// This elides the need to clone or copy data.
pub fn throughput_for_each(callback: &mut dyn FnMut(&XdpIpAddress, &[HostCounter])) {
    unsafe {
        crate::bpf_iterator::iterate_throughput(callback);
    }
}

/// Reads aggregate host counter map pressure from the eBPF datapath.
///
/// Side effects: opens the pinned `/sys/fs/bpf/map_traffic_pressure` BPF map.
pub fn traffic_map_pressure() -> anyhow::Result<TrafficMapPressure> {
    let path = CString::new("/sys/fs/bpf/map_traffic_pressure")?;
    let fd = unsafe { libbpf_sys::bpf_obj_get(path.as_ptr()) };
    if fd < 0 {
        return Err(anyhow::Error::msg("Unable to open map_traffic_pressure"));
    }

    let cpu_count = crate::num_possible_cpus()? as usize;
    let mut key = 0_u32;
    let mut values = vec![TrafficMapPressure::default(); cpu_count];
    let err = unsafe {
        libbpf_sys::bpf_map_lookup_elem(
            fd,
            &mut key as *mut u32 as *mut c_void,
            values.as_mut_ptr() as *mut c_void,
        )
    };
    unsafe {
        nix::libc::close(fd);
    }
    if err != 0 {
        return Err(anyhow::Error::msg(format!(
            "Unable to read map_traffic_pressure ({err})"
        )));
    }

    Ok(values
        .into_iter()
        .fold(TrafficMapPressure::default(), |mut total, pressure| {
            total.insert_failures = total
                .insert_failures
                .saturating_add(pressure.insert_failures);
            total.last_failure_ns = total.last_failure_ns.max(pressure.last_failure_ns);
            total
        }))
}

#[cfg(test)]
mod test {
    use super::{HostCounter, TrafficMapPressure};

    #[test]
    fn host_counter_size() {
        assert_eq!(std::mem::size_of::<HostCounter>(), 128);
    }

    #[test]
    fn traffic_map_pressure_size() {
        assert_eq!(std::mem::size_of::<TrafficMapPressure>(), 16);
    }
}
