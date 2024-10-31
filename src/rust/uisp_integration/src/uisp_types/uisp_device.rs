use crate::ip_ranges::IpRanges;
use lqos_config::Config;
use std::collections::HashSet;
use std::net::IpAddr;
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
    pub download: u32,
    pub upload: u32,
    pub ipv4: HashSet<String>,
    pub ipv6: HashSet<String>,
}

impl UispDevice {
    /// Creates a new UispDevice from a UISP device
    /// 
    /// # Arguments
    /// * `device` - The device to convert
    /// * `config` - The configuration
    /// * `ip_ranges` - The IP ranges to use for the network
    pub fn from_uisp(device: &Device, config: &Config, ip_ranges: &IpRanges, ipv4_to_v6: &[Ipv4ToIpv6]) -> Self {
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
                download = dl as u32 / 1000000;
            }
            if let Some(ul) = overview.uplinkCapacity {
                upload = ul as u32 / 1000000;
            }
        }

        // Accumulate IP address listings
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
                                // We can't trust UISP to provide the correct suffix, so change that to /32
                                if address.contains('/') {
                                    let splits: Vec<_> = address.split('/').collect();
                                    ipv4.insert(format!("{}/32", splits[0]));
                                    
                                    // Check for a Mikrotik Mapping
                                    if let Some(mapping) = ipv4_to_v6.iter().find(|m| m.ipv4 == splits[0]) {
                                        ipv6.insert(mapping.ipv6.clone());
                                    }
                                } else {
                                    ipv4.insert(format!("{address}/32"));

                                    // Check for a Mikrotik Mapping
                                    if let Some(mapping) = ipv4_to_v6.iter().find(|m| m.ipv4 == address.as_str()) {
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
        let mut site_id = device.get_site_id().unwrap_or("".to_string());
        for exception in config.uisp_integration.exception_cpes.iter() {
            if exception.cpe == device.get_name().unwrap() {
                site_id = exception.parent.clone();
            }
        }

        Self {
            id: device.get_id(),
            name: device.get_name().unwrap(),
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
