use anyhow::Result;
use lqos_bus::TcHandle;
use std::net::IpAddr;
use crate::{bpf_map::BpfMap, XdpIpAddress};
mod ip_to_map;
mod ip_hash_data;
mod ip_hash_key;
use ip_to_map::IpToMap;
use ip_hash_data::IpHashData;
use ip_hash_key::IpHashKey;

/// Adds an IP address to the underlying TC map.
/// 
/// ## Arguments
/// 
/// * `address` - a string containing an IPv4 or IPv6 address, with or without a prefix-length.
/// * `tc_handle` - the TC classifier handle to associate with the IP address, in (major,minor) format.
/// * `cpu` - the CPU index on which the TC class should be handled.
pub fn add_ip_to_tc(address: &str, tc_handle: TcHandle, cpu: u32, upload: bool) -> Result<()> {
    let bpf_path = if upload {
        "/sys/fs/bpf/map_ip_to_cpu_and_tc_recip"
    } else {
        "/sys/fs/bpf/map_ip_to_cpu_and_tc"
    };

    let ip_to_add = IpToMap::new(address, tc_handle, cpu)?;
    let mut bpf_map =
        BpfMap::<IpHashKey, IpHashData>::from_path(bpf_path)?;
    let address = XdpIpAddress::from_ip(ip_to_add.subnet);
    let mut key = IpHashKey {
        prefixlen: ip_to_add.prefix,
        address: address.0,
    };
    let mut value = IpHashData {
        cpu: ip_to_add.cpu,
        tc_handle: ip_to_add.handle(),
    };
    bpf_map.insert(&mut key, &mut value)?;
    Ok(())
}

/// Removes an IP address from the underlying TC map.
/// 
/// ## Arguments
/// 
/// * `address` - the IP address to remove. If no prefix (e.g. `/24`) is provided, the longest prefix to match a single IP address will be assumed.
pub fn del_ip_from_tc(address: &str, upload: bool) -> Result<()> {
    let bpf_path = if upload {
        "/sys/fs/bpf/map_ip_to_cpu_and_tc_recip"
    } else {
        "/sys/fs/bpf/map_ip_to_cpu_and_tc"
    };
    let ip_to_add = IpToMap::new(address, TcHandle::from_string("0:0")?, 0)?;
    let mut bpf_map =
        BpfMap::<IpHashKey, IpHashData>::from_path(bpf_path)?;
    let ip = address.parse::<IpAddr>()?;
    let ip = XdpIpAddress::from_ip(ip);
    let mut key = IpHashKey {
        prefixlen: ip_to_add.prefix,
        address: ip.0,
    };
    bpf_map.delete(&mut key)?;
    Ok(())
}

/// Remove all IP addresses from the underlying TC map.
pub fn clear_ips_from_tc() -> Result<()> {
    let mut bpf_map =
        BpfMap::<IpHashKey, IpHashData>::from_path("/sys/fs/bpf/map_ip_to_cpu_and_tc")?;
    bpf_map.clear()?;

    let mut bpf_map =
        BpfMap::<IpHashKey, IpHashData>::from_path("/sys/fs/bpf/map_ip_to_cpu_and_tc_recip")?;
    bpf_map.clear()?;

    Ok(())
}

/// Query the underlying IP address to TC map and return the currently active dataset.
pub fn list_mapped_ips() -> Result<Vec<(IpHashKey, IpHashData)>> {
    let bpf_map = BpfMap::<IpHashKey, IpHashData>::from_path("/sys/fs/bpf/map_ip_to_cpu_and_tc")?;
    let mut raw = bpf_map.dump_vec();

    let bpf_map2 = BpfMap::<IpHashKey, IpHashData>::from_path("/sys/fs/bpf/map_ip_to_cpu_and_tc_recip")?;
    let raw2 = bpf_map2.dump_vec();
    raw.extend_from_slice(&raw2);

    Ok(raw)
}
