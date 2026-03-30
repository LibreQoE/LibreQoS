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
    pub role: Option<String>,
    pub wireless_mode: Option<String>,
    pub site_id: String,
    pub download: u64,
    pub upload: u64,
    pub ipv4: HashSet<String>,
    pub ipv6: HashSet<String>,
    pub negotiated_ethernet_mbps: Option<u64>,
    pub negotiated_ethernet_interface: Option<String>,
}

impl UispDevice {
    fn parse_speed_mbps(raw: &str) -> Option<u64> {
        let lower = raw.trim().to_lowercase();
        if lower.is_empty() {
            return None;
        }

        let digits: String = lower
            .chars()
            .take_while(|ch| ch.is_ascii_digit() || *ch == '.')
            .collect();
        if digits.is_empty() {
            return None;
        }

        let value = digits.parse::<f64>().ok()?;
        if !(value.is_finite() && value > 0.0) {
            return None;
        }

        let multiplier = if lower.contains("gbps") || lower.ends_with('g') {
            1000.0
        } else {
            1.0
        };

        Some((value * multiplier).round() as u64)
    }

    fn negotiated_ethernet_from_device(device: &Device) -> (Option<u64>, Option<String>) {
        let mut best: Option<(u64, Option<String>)> = None;
        let Some(interfaces) = &device.interfaces else {
            return (None, None);
        };

        for interface in interfaces {
            let interface_type = interface
                .identification
                .as_ref()
                .and_then(|id| id.r#type.clone())
                .unwrap_or_default()
                .to_lowercase();
            if matches!(interface_type.as_str(), "wlan" | "wifi" | "wireless") {
                continue;
            }
            let Some(status) = &interface.status else {
                continue;
            };
            let speed_raw = if let Some(current_speed) = status.currentSpeed.as_deref() {
                Some(current_speed)
            } else if Self::allows_plain_speed(status.status.as_deref()) {
                status.speed.as_deref()
            } else {
                None
            };
            let Some(speed_raw) = speed_raw else {
                continue;
            };
            let Some(speed_mbps) = Self::parse_speed_mbps(speed_raw) else {
                continue;
            };
            let iface_name = interface
                .identification
                .as_ref()
                .and_then(|id| id.name.clone());

            if best
                .as_ref()
                .is_none_or(|(current_speed, _)| speed_mbps < *current_speed)
            {
                best = Some((speed_mbps, iface_name));
            }
        }

        best.map_or((None, None), |(speed, iface)| (Some(speed), iface))
    }

    fn allows_plain_speed(status: Option<&str>) -> bool {
        let Some(status) = status else {
            return false;
        };
        matches!(
            status.trim().to_ascii_lowercase().as_str(),
            "connected" | "up" | "active"
        )
    }

    pub(crate) fn is_wireless_station_cpe(&self) -> bool {
        let role = self
            .role
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase);
        if matches!(role.as_deref(), Some("station" | "sta" | "cpe")) {
            return true;
        }

        let wireless_mode = self
            .wireless_mode
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase);
        matches!(
            wireless_mode.as_deref(),
            Some(mode) if mode.starts_with("sta") || mode.starts_with("station")
        )
    }

    pub(crate) fn is_router_like(&self) -> bool {
        let role = self
            .role
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase);
        matches!(
            role.as_deref(),
            Some("router" | "homewifi" | "home-wifi" | "home wifi")
        )
    }

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
        if prefix_len > 32 {
            return false; // invalid prefix for IPv4
        }

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

        // Build a host mask safely; shifting by 32 would overflow, so special-case /0
        let host_mask = if host_bits >= 32 {
            u32::MAX
        } else {
            (1u32 << host_bits) - 1
        };

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
        let (negotiated_ethernet_mbps, negotiated_ethernet_interface) =
            Self::negotiated_ethernet_from_device(device);

        Self {
            id: device.get_id(),
            name: device.get_name().unwrap_or_default(),
            mac,
            role: device.identification.role.clone(),
            wireless_mode: device
                .overview
                .as_ref()
                .and_then(|o| o.wirelessMode.clone()),
            site_id,
            upload,
            download,
            ipv4,
            ipv6,
            negotiated_ethernet_mbps,
            negotiated_ethernet_interface,
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
    use serde_json::json;
    use uisp::Device;

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
        assert!(UispDevice::is_network_address("0.0.0.0/0")); // Only 0.0.0.0/0 is a network address for /0

        // Host addresses (should return false)
        assert!(!UispDevice::is_network_address("192.168.1.1/24"));
        assert!(!UispDevice::is_network_address("192.168.1.255/24"));
        assert!(!UispDevice::is_network_address("10.0.0.1/8"));
        assert!(!UispDevice::is_network_address("172.16.0.1/16"));
        assert!(!UispDevice::is_network_address("23.148.208.225/29")); // 225 = 11100001, host in /29
        assert!(!UispDevice::is_network_address("5.5.6.1/24"));
        assert!(!UispDevice::is_network_address("192.168.0.1/23"));
        assert!(!UispDevice::is_network_address("10.10.10.129/25")); // 129 = 10000001, host in /25
        assert!(!UispDevice::is_network_address("1.2.3.4/0")); // /0 but not all host bits zero

        // Special cases
        assert!(!UispDevice::is_network_address("192.168.1.1/32")); // /32 is always a host
        assert!(!UispDevice::is_network_address("192.168.1.0/32")); // /32 is always a host
        assert!(!UispDevice::is_network_address("192.168.1.1")); // No CIDR
        assert!(!UispDevice::is_network_address("")); // Empty string
        assert!(!UispDevice::is_network_address("not-an-ip/24")); // Invalid IP
        assert!(!UispDevice::is_network_address("192.168.1.0/invalid")); // Invalid prefix
        assert!(!UispDevice::is_network_address("192.168.1.0/33")); // Invalid IPv4 prefix length
    }

    #[test]
    fn test_parse_speed_mbps() {
        assert_eq!(UispDevice::parse_speed_mbps("100Mbps-Full"), Some(100));
        assert_eq!(UispDevice::parse_speed_mbps("1000"), Some(1000));
        assert_eq!(UispDevice::parse_speed_mbps("1Gbps"), Some(1000));
        assert_eq!(UispDevice::parse_speed_mbps("2.5Gbps"), Some(2500));
        assert_eq!(UispDevice::parse_speed_mbps(""), None);
        assert_eq!(UispDevice::parse_speed_mbps("unknown"), None);
    }

    fn mk_device_with_interfaces(interfaces: serde_json::Value) -> Device {
        serde_json::from_value(json!({
            "identification": {
                "id": "dev-1",
                "hostname": "dev-1",
                "role": null,
            },
            "overview": {
                "wirelessMode": null
            },
            "interfaces": interfaces,
        }))
        .expect("device JSON must deserialize")
    }

    #[test]
    fn disconnected_plain_speed_is_ignored_for_negotiated_ethernet() {
        let device = mk_device_with_interfaces(json!([
            {
                "identification": { "name": "eth0@1", "type": "eth" },
                "status": { "status": "disconnected", "speed": "10-half", "currentSpeed": null },
                "wireless": {}
            }
        ]));

        let (speed, iface) = UispDevice::negotiated_ethernet_from_device(&device);
        assert_eq!(speed, None);
        assert_eq!(iface, None);
    }

    #[test]
    fn disconnected_currentspeed_is_still_accepted() {
        let device = mk_device_with_interfaces(json!([
            {
                "identification": { "name": "data", "type": "eth" },
                "status": { "status": "disconnected", "speed": "auto", "currentSpeed": "100-full" },
                "wireless": {}
            }
        ]));

        let (speed, iface) = UispDevice::negotiated_ethernet_from_device(&device);
        assert_eq!(speed, Some(100));
        assert_eq!(iface.as_deref(), Some("data"));
    }

    #[test]
    fn connected_plain_speed_is_accepted() {
        let device = mk_device_with_interfaces(json!([
            {
                "identification": { "name": "eth0", "type": "eth" },
                "status": { "status": "connected", "speed": "1000-full", "currentSpeed": null },
                "wireless": {}
            }
        ]));

        let (speed, iface) = UispDevice::negotiated_ethernet_from_device(&device);
        assert_eq!(speed, Some(1000));
        assert_eq!(iface.as_deref(), Some("eth0"));
    }
}
