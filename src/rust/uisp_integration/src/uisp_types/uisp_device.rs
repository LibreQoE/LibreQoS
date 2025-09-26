use crate::ip_ranges::IpRanges;
use lqos_config::Config;
use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr};
use uisp::Device;

#[derive(Debug)]
pub struct Ipv4ToIpv6 {
    pub ipv4: String,
    pub ipv6: String,
}

/// Trimmed UISP device for easy use
pub struct UispDevice {
    pub id: String,
    pub name: String,
    pub mac: String,
    pub site_id: String,
    pub download: u64,
    pub upload: u64,
    pub ipv4: HashSet<String>,
    pub ipv6: HashSet<String>,
}

impl UispDevice {
    /// Check if an IP/CIDR represents a network address (all host bits are zero)
    /// Returns true if it's a network address, false if it's a host address
    #[cfg_attr(test, allow(dead_code))]
    pub fn is_network_address(cidr: &str) -> bool {
        if !cidr.contains('/') {
            return false;
        }

        let parts: Vec<&str> = cidr.split('/').collect();
        if parts.len() != 2 {
            return false;
        }

        let ip_str = parts[0];
        let prefix_len: u32 = match parts[1].parse() {
            Ok(p) => p,
            Err(_) => return false,
        };

        // Parse the IP address
        let ip: Ipv4Addr = match ip_str.parse() {
            Ok(addr) => addr,
            Err(_) => return false,
        };

        // Convert IP to u32 for bit manipulation
        let ip_bits = u32::from(ip);

        // Calculate the host mask (bits that should be zero for a network address)
        let host_bits = 32 - prefix_len;
        if host_bits == 0 {
            // /32 is always treated as a host address
            return false;
        }

        let host_mask = (1u32 << host_bits) - 1;

        // Check if all host bits are zero
        (ip_bits & host_mask) == 0
    }

    /// Creates a new UispDevice from a UISP device
    ///
    /// # Arguments
    /// * `device` - The device to convert
    /// * `config` - The configuration
    /// * `ip_ranges` - The IP ranges to use for the network
    pub fn from_uisp(
        device: &Device,
        config: &Config,
        ip_ranges: &IpRanges,
        ipv4_to_v6: &[Ipv4ToIpv6],
    ) -> Self {
        let mut ipv4 = HashSet::new();
        let mut ipv6 = HashSet::new();
        let mac = if let Some(id) = &device.identification.mac {
            id.clone()
        } else {
            "".to_string()
        };

        let mut download = config.queues.generated_pn_download_mbps;
        let mut upload = config.queues.generated_pn_upload_mbps;
        if let Some(overview) = &device.overview {
            if let Some(dl) = overview.downlinkCapacity {
                download = dl as u64 / 1000000;
            }
            if let Some(ul) = overview.uplinkCapacity {
                upload = ul as u64 / 1000000;
            }
            if device.get_model().unwrap_or_default().contains("5AC") {
                download =
                    ((download as f64) * config.uisp_integration.airmax_capacity as f64) as u64;
                upload = ((upload as f64) * config.uisp_integration.airmax_capacity as f64) as u64;
            }
            if device.get_model().unwrap_or_default().contains("LTU") {
                download = ((download as f64) * config.uisp_integration.ltu_capacity as f64) as u64;
                upload = ((upload as f64) * config.uisp_integration.ltu_capacity as f64) as u64;
            }
        }
        if download == 0 {
            download = config.queues.generated_pn_download_mbps;
        }
        if upload == 0 {
            upload = config.queues.generated_pn_upload_mbps;
        }

        // Process the single ipAddress field (never includes CIDR, always /32)
        if let Some(ip) = &device.ipAddress {
            if ip.contains(':') {
                // It's IPv6
                ipv6.insert(ip.clone());
            } else {
                // It's IPv4 - always add as /32
                let base_ip = if ip.contains('/') {
                    ip.split('/').next().unwrap_or(ip)
                } else {
                    ip.as_str()
                };
                ipv4.insert(format!("{}/32", base_ip));

                // Check for a Mikrotik Mapping
                if let Some(mapping) = ipv4_to_v6.iter().find(|m| m.ipv4 == base_ip) {
                    ipv6.insert(mapping.ipv6.clone());
                }
            }
        }

        // Accumulate IP address listings from interfaces
        if let Some(interfaces) = &device.interfaces {
            for interface in interfaces.iter() {
                if let Some(addr) = &interface.addresses {
                    for address in addr.iter() {
                        if let Some(address) = &address.cidr {
                            if address.contains(':') {
                                // It's IPv6
                                ipv6.insert(address.clone());
                            } else {
                                // It's IPv4
                                // Check if this is a network address or host address
                                if address.contains('/') {
                                    let splits: Vec<_> = address.split('/').collect();
                                    let base_ip = splits[0];

                                    // If it's a network address (e.g., 5.5.6.0/24), keep the CIDR
                                    // If it's a host address (e.g., 5.5.6.1/24), make it /32
                                    let final_address = if Self::is_network_address(address) {
                                        address.clone()
                                    } else {
                                        format!("{}/32", base_ip)
                                    };

                                    ipv4.insert(final_address);

                                    // Check for a Mikrotik Mapping
                                    if let Some(mapping) =
                                        ipv4_to_v6.iter().find(|m| m.ipv4 == base_ip)
                                    {
                                        ipv6.insert(mapping.ipv6.clone());
                                    }
                                } else {
                                    ipv4.insert(format!("{address}/32"));

                                    // Check for a Mikrotik Mapping
                                    if let Some(mapping) =
                                        ipv4_to_v6.iter().find(|m| m.ipv4 == address.as_str())
                                    {
                                        ipv6.insert(mapping.ipv6.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Remove IP addresses that are disallowed
        ipv4.retain(|ip| {
            let split: Vec<_> = ip.split('/').collect();
            //let subnet: u8 = split[1].parse().unwrap();
            let addr: IpAddr = split[0].parse().unwrap();
            ip_ranges.is_permitted(addr)
        });
        ipv6.retain(|ip| {
            let split: Vec<_> = ip.split('/').collect();
            //let subnet: u8 = split[1].parse().unwrap();
            let addr: IpAddr = split[0].parse().unwrap();
            ip_ranges.is_permitted(addr)
        });

        // Handle any "exception CPE" entries
        let mut site_id = device.get_site_id().unwrap_or_default();
        for exception in config.uisp_integration.exception_cpes.iter() {
            if exception.cpe == device.get_name().unwrap_or_default() {
                site_id = exception.parent.clone();
            }
        }

        Self {
            id: device.get_id(),
            name: device.get_name().unwrap_or_default(),
            mac,
            site_id,
            upload,
            download,
            ipv4,
            ipv6,
        }
    }

    pub fn has_address(&self) -> bool {
        !(self.ipv4.is_empty() && self.ipv6.is_empty())
    }

    pub fn ipv4_list(&self) -> String {
        if self.ipv4.is_empty() {
            return "".to_string();
        }
        if self.ipv4.len() == 1 {
            let mut result = "".to_string();
            for ip in self.ipv4.iter() {
                result = ip.clone();
            }
            return result;
        }
        let mut result = "".to_string();
        for ip in self.ipv4.iter() {
            result += &format!("{}, ", &ip);
        }
        result.truncate(result.len() - 2);
        result.to_string()
    }

    pub fn ipv6_list(&self) -> String {
        if self.ipv6.is_empty() {
            return "".to_string();
        }
        if self.ipv6.len() == 1 {
            let mut result = "".to_string();
            for ip in self.ipv6.iter() {
                result = ip.clone();
            }
            return result;
        }
        let mut result = "".to_string();
        for ip in self.ipv6.iter() {
            result += &format!("{}, ", &ip);
        }
        result.truncate(result.len() - 2);
        let result = format!("[{result}]");
        result
    }
}

#[cfg(test)]
mod tests {
    use super::UispDevice;

    #[test]
    fn test_is_network_address() {
        // Network addresses (should return true)
        assert!(UispDevice::is_network_address("192.168.1.0/24"));
        assert!(UispDevice::is_network_address("10.0.0.0/8"));
        assert!(UispDevice::is_network_address("172.16.0.0/16"));
        assert!(UispDevice::is_network_address("23.148.208.224/29")); // 224 = 11100000, network for /29
        assert!(UispDevice::is_network_address("5.5.6.0/24"));
        assert!(UispDevice::is_network_address("192.168.0.0/23"));
        assert!(UispDevice::is_network_address("10.10.10.128/25")); // 128 = 10000000, network for /25

        // Host addresses (should return false)
        assert!(!UispDevice::is_network_address("192.168.1.1/24"));
        assert!(!UispDevice::is_network_address("192.168.1.255/24"));
        assert!(!UispDevice::is_network_address("10.0.0.1/8"));
        assert!(!UispDevice::is_network_address("172.16.0.1/16"));
        assert!(!UispDevice::is_network_address("23.148.208.225/29")); // 225 = 11100001, host in /29
        assert!(!UispDevice::is_network_address("5.5.6.1/24"));
        assert!(!UispDevice::is_network_address("192.168.0.1/23"));
        assert!(!UispDevice::is_network_address("10.10.10.129/25")); // 129 = 10000001, host in /25

        // Special cases
        assert!(!UispDevice::is_network_address("192.168.1.1/32")); // /32 is always a host
        assert!(!UispDevice::is_network_address("192.168.1.0/32")); // /32 is always a host
        assert!(!UispDevice::is_network_address("192.168.1.1")); // No CIDR
        assert!(!UispDevice::is_network_address("")); // Empty string
        assert!(!UispDevice::is_network_address("not-an-ip/24")); // Invalid IP
        assert!(!UispDevice::is_network_address("192.168.1.0/invalid")); // Invalid prefix
    }
}
