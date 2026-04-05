use crate::ip_ranges::IpRanges;
use lqos_config::{Config, EthernetPortLimitPolicy, usable_ethernet_cap_mbps};
use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr};
use uisp::Device;

#[derive(Debug)]
pub struct Ipv4ToIpv6 {
    pub ipv4: String,
    pub ipv6: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UispAttachmentRateSource {
    DynamicIntegration,
    Static,
}

/// Trimmed UISP device for easy use
pub struct UispDevice {
    pub id: String,
    pub name: String,
    pub mac: String,
    pub role: Option<String>,
    pub wireless_mode: Option<String>,
    pub site_id: String,
    pub raw_download: u64,
    pub raw_upload: u64,
    pub download: u64,
    pub upload: u64,
    pub ipv4: HashSet<String>,
    pub ipv6: HashSet<String>,
    pub probe_ipv4: HashSet<String>,
    pub probe_ipv6: HashSet<String>,
    pub negotiated_ethernet_mbps: Option<u64>,
    pub negotiated_ethernet_interface: Option<String>,
    pub transport_cap_mbps: Option<u64>,
    pub transport_cap_reason: Option<String>,
    pub attachment_rate_source: UispAttachmentRateSource,
}

impl UispDevice {
    fn mbps_from_bps(raw: Option<i64>) -> Option<f64> {
        let value = raw?;
        if value <= 0 {
            return None;
        }
        Some(value as f64 / 1_000_000.0)
    }

    fn normalize_download_ratio(raw: f64) -> Option<f64> {
        if !raw.is_finite() || raw <= 0.0 {
            return None;
        }
        if raw < 1.0 {
            return Some(raw);
        }
        if raw <= 100.0 {
            return Some(raw / 100.0);
        }
        None
    }

    fn active_wireless_dl_ratio(device: &Device) -> Option<f64> {
        let Some(interfaces) = &device.interfaces else {
            return None;
        };

        let active_interface_names = device
            .overview
            .as_ref()
            .and_then(|overview| overview.wirelessActiveInterfaceIds.as_ref());

        if let Some(active_interface_names) = active_interface_names {
            for active_interface_name in active_interface_names {
                let ratio = interfaces.iter().find_map(|interface| {
                    let identification = interface.identification.as_ref()?;
                    if identification.name.as_deref() != Some(active_interface_name.as_str()) {
                        return None;
                    }
                    interface.wireless.as_ref()?.dlRatio
                });
                if let Some(ratio) = ratio.and_then(Self::normalize_download_ratio) {
                    return Some(ratio);
                }
            }
        }

        interfaces
            .iter()
            .filter(|interface| {
                let Some(identification) = &interface.identification else {
                    return false;
                };
                identification.r#type.as_deref().is_some_and(|kind| {
                    matches!(
                        kind.to_ascii_lowercase().as_str(),
                        "wlan" | "wifi" | "wireless"
                    )
                })
            })
            .find_map(|interface| interface.wireless.as_ref()?.dlRatio)
            .and_then(Self::normalize_download_ratio)
    }

    fn is_airmax_ap(device: &Device) -> bool {
        device
            .identification
            .r#type
            .as_deref()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("airmax"))
            && device
                .identification
                .role
                .as_deref()
                .is_some_and(|role| role.eq_ignore_ascii_case("ap"))
    }

    fn is_airmax_flexible_frame_ap(device: &Device) -> bool {
        if !Self::is_airmax_ap(device) {
            return false;
        }

        let Some(overview) = &device.overview else {
            return false;
        };

        let is_ptmp_ap = overview
            .wirelessMode
            .as_deref()
            .is_some_and(|mode| mode.eq_ignore_ascii_case("ap-ptmp"));
        if !is_ptmp_ap {
            return false;
        }

        Self::active_wireless_dl_ratio(device).is_some()
            || overview
                .theoreticalTotalCapacity
                .is_some_and(|capacity| capacity > 0)
    }

    fn airmax_ap_capacities(device: &Device, config: &Config) -> Option<(u64, u64)> {
        if !Self::is_airmax_flexible_frame_ap(device) {
            return None;
        }

        let overview = device.overview.as_ref()?;
        let total_capacity_mbps = overview
            .totalCapacity
            .and_then(|capacity| Self::mbps_from_bps(Some(capacity)))
            .or_else(|| {
                let downlink = Self::mbps_from_bps(overview.downlinkCapacity)?;
                let uplink = Self::mbps_from_bps(overview.uplinkCapacity)?;
                Some(downlink.max(uplink))
            })?
            * config.uisp_integration.airmax_capacity as f64;
        if !(total_capacity_mbps.is_finite() && total_capacity_mbps > 0.0) {
            return None;
        }

        let download_ratio = Self::active_wireless_dl_ratio(device)
            .unwrap_or(config.uisp_integration.airmax_flexible_frame_download_ratio as f64);
        let upload_ratio = 1.0 - download_ratio;
        if !(upload_ratio.is_finite() && upload_ratio > 0.0) {
            return None;
        }

        Some((
            (total_capacity_mbps * download_ratio).round() as u64,
            (total_capacity_mbps * upload_ratio).round() as u64,
        ))
    }

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

    fn normalized_model_key(device: &Device) -> String {
        device
            .get_model()
            .unwrap_or_default()
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .map(|ch| ch.to_ascii_uppercase())
            .collect()
    }

    fn is_transport_interface(interface_type: Option<&str>, interface_name: Option<&str>) -> bool {
        let interface_type = interface_type
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        if matches!(interface_type.as_str(), "wlan" | "wifi" | "wireless") {
            return false;
        }
        if matches!(
            interface_type.as_str(),
            "eth" | "ethernet" | "sfp" | "sfp+" | "sfpplus" | "fiber" | "optical"
        ) {
            return true;
        }

        let interface_name = interface_name
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        interface_name.starts_with("eth")
            || interface_name.starts_with("sfp")
            || interface_name.starts_with("fiber")
    }

    fn infrastructure_transport_observation(
        device: &Device,
    ) -> (Option<u64>, Option<String>, Option<String>) {
        let Some(interfaces) = &device.interfaces else {
            return (None, None, None);
        };

        let mut observed = Vec::<(u64, Option<String>)>::new();
        for interface in interfaces {
            let interface_type = interface
                .identification
                .as_ref()
                .and_then(|id| id.r#type.as_deref());
            let interface_name = interface
                .identification
                .as_ref()
                .and_then(|id| id.name.as_deref());
            if !Self::is_transport_interface(interface_type, interface_name) {
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
            observed.push((speed_mbps, iface_name));
        }

        let Some((speed, iface_name)) = observed.into_iter().max_by_key(|(speed, _)| *speed) else {
            return (None, None, None);
        };
        if speed == 0 {
            return (None, None, None);
        }
        let reason = iface_name
            .as_ref()
            .map(|iface| format!("Active transport interface {iface} negotiated at {speed} Mbps"))
            .unwrap_or_else(|| format!("Active transport interface negotiated at {speed} Mbps"));
        (Some(speed), iface_name, Some(reason))
    }

    fn model_transport_port_ceiling(device: &Device) -> Option<(u64, String)> {
        match Self::normalized_model_key(device).as_str() {
            "AF60LR" => Some((
                1000,
                "Model AF60LR is limited to 1G Ethernet ports".to_string(),
            )),
            _ => None,
        }
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

    fn has_dynamic_radio_capacity(device: &Device) -> bool {
        let Some(overview) = &device.overview else {
            return false;
        };
        if overview.wirelessMode.is_none() {
            return false;
        }
        overview.downlinkCapacity.is_some()
            || overview.uplinkCapacity.is_some()
            || overview.totalCapacity.is_some()
            || overview.theoreticalTotalCapacity.is_some()
    }

    fn probe_ip_candidate(raw: &str) -> Option<String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }

        if trimmed.contains(':') {
            let addr = trimmed
                .split('/')
                .next()
                .unwrap_or(trimmed)
                .parse::<IpAddr>()
                .ok()?;
            let IpAddr::V6(addr) = addr else {
                return None;
            };
            if addr.is_unspecified() || addr.is_multicast() || addr.is_unicast_link_local() {
                return None;
            }
            return Some(addr.to_string());
        }

        if trimmed.contains('/') && Self::is_network_address(trimmed) {
            return None;
        }

        let addr = trimmed
            .split('/')
            .next()
            .unwrap_or(trimmed)
            .parse::<IpAddr>()
            .ok()?;
        let IpAddr::V4(addr) = addr else {
            return None;
        };
        if addr.is_unspecified() || addr.is_multicast() {
            return None;
        }
        Some(addr.to_string())
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
        let mut probe_ipv4 = HashSet::new();
        let mut probe_ipv6 = HashSet::new();
        let mac = if let Some(id) = &device.identification.mac {
            id.clone()
        } else {
            "".to_string()
        };

        let mut download = config.queues.generated_pn_download_mbps;
        let mut upload = config.queues.generated_pn_upload_mbps;
        let mut attachment_rate_source = UispAttachmentRateSource::Static;
        if let Some(overview) = &device.overview {
            if let Some((airmax_download, airmax_upload)) =
                Self::airmax_ap_capacities(device, config)
            {
                download = airmax_download;
                upload = airmax_upload;
                attachment_rate_source = UispAttachmentRateSource::DynamicIntegration;
            } else {
                if let Some(dl) = overview.downlinkCapacity {
                    download = dl as u64 / 1000000;
                }
                if let Some(ul) = overview.uplinkCapacity {
                    upload = ul as u64 / 1000000;
                }
                if Self::has_dynamic_radio_capacity(device) {
                    attachment_rate_source = UispAttachmentRateSource::DynamicIntegration;
                }
                if device.get_model().unwrap_or_default().contains("5AC") {
                    download =
                        ((download as f64) * config.uisp_integration.airmax_capacity as f64) as u64;
                    upload =
                        ((upload as f64) * config.uisp_integration.airmax_capacity as f64) as u64;
                }
                if device.get_model().unwrap_or_default().contains("LTU") {
                    download =
                        ((download as f64) * config.uisp_integration.ltu_capacity as f64) as u64;
                    upload = ((upload as f64) * config.uisp_integration.ltu_capacity as f64) as u64;
                }
            }
        }
        if download == 0 {
            download = config.queues.generated_pn_download_mbps;
        }
        if upload == 0 {
            upload = config.queues.generated_pn_upload_mbps;
        }
        let raw_download = download;
        let raw_upload = upload;
        let mut transport_cap_mbps = None;
        let mut transport_cap_reason = None;

        // Process the single ipAddress field (never includes CIDR, always /32)
        if let Some(ip) = &device.ipAddress {
            if ip.contains(':') {
                // It's IPv6
                ipv6.insert(ip.clone());
                if let Some(probe_ip) = Self::probe_ip_candidate(ip) {
                    probe_ipv6.insert(probe_ip);
                }
            } else {
                // It's IPv4 - always add as /32
                let base_ip = if ip.contains('/') {
                    ip.split('/').next().unwrap_or(ip)
                } else {
                    ip.as_str()
                };
                ipv4.insert(format!("{}/32", base_ip));
                if let Some(probe_ip) = Self::probe_ip_candidate(ip) {
                    probe_ipv4.insert(probe_ip);
                }

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
                                if let Some(probe_ip) = Self::probe_ip_candidate(address) {
                                    probe_ipv6.insert(probe_ip);
                                }
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
                                    if let Some(probe_ip) = Self::probe_ip_candidate(address) {
                                        probe_ipv4.insert(probe_ip);
                                    }

                                    // Check for a Mikrotik Mapping
                                    if let Some(mapping) =
                                        ipv4_to_v6.iter().find(|m| m.ipv4 == base_ip)
                                    {
                                        ipv6.insert(mapping.ipv6.clone());
                                    }
                                } else {
                                    ipv4.insert(format!("{address}/32"));
                                    if let Some(probe_ip) = Self::probe_ip_candidate(address) {
                                        probe_ipv4.insert(probe_ip);
                                    }

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
        if config
            .uisp_integration
            .infrastructure_transport_caps_enabled
        {
            let policy = EthernetPortLimitPolicy::from(&config.integration_common);
            let observed_transport = Self::infrastructure_transport_observation(device);
            let model_transport = Self::model_transport_port_ceiling(device);
            let selected_transport = observed_transport
                .0
                .zip(observed_transport.2)
                .or(model_transport);

            if let Some((line_rate_mbps, reason)) = selected_transport
                && let Some(usable_cap_mbps) = usable_ethernet_cap_mbps(policy, line_rate_mbps)
                && (download > usable_cap_mbps || upload > usable_cap_mbps)
            {
                download = download.min(usable_cap_mbps);
                upload = upload.min(usable_cap_mbps);
                transport_cap_mbps = Some(usable_cap_mbps);
                transport_cap_reason = Some(reason);
            }
        }

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
            raw_download,
            raw_upload,
            upload,
            download,
            ipv4,
            ipv6,
            probe_ipv4,
            probe_ipv6,
            negotiated_ethernet_mbps,
            negotiated_ethernet_interface,
            transport_cap_mbps,
            transport_cap_reason,
            attachment_rate_source,
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
    use crate::ip_ranges::IpRanges;
    use lqos_config::Config;
    use serde_json::json;
    use std::collections::HashSet;
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
                "type": null,
            },
            "overview": {
                "wirelessMode": null
            },
            "interfaces": interfaces,
        }))
        .expect("device JSON must deserialize")
    }

    fn test_ip_ranges() -> IpRanges {
        IpRanges::new(&Config::default()).expect("test config should build ip ranges")
    }

    fn test_config() -> Config {
        let mut config = Config::default();
        config.uisp_integration.airmax_capacity = 1.0;
        config.uisp_integration.airmax_flexible_frame_download_ratio = 0.8;
        config
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

    #[test]
    fn airmax_ptmp_ap_uses_reported_total_capacity_and_dl_ratio() {
        let device: Device = serde_json::from_value(json!({
            "identification": {
                "id": "airmax-ap-1",
                "hostname": "airmax-ap-1",
                "model": "RP-5AC-Gen2",
                "role": "ap",
                "type": "airMax"
            },
            "overview": {
                "wirelessMode": "ap-ptmp",
                "totalCapacity": 200000000,
                "theoreticalTotalCapacity": 1000000,
                "wirelessActiveInterfaceIds": ["main"]
            },
            "interfaces": [
                {
                    "identification": { "name": "main", "type": "wlan" },
                    "wireless": { "dlRatio": 75.0 }
                }
            ]
        }))
        .expect("device JSON must deserialize");

        let trimmed = UispDevice::from_uisp(&device, &test_config(), &test_ip_ranges(), &[]);
        assert_eq!(trimmed.download, 150);
        assert_eq!(trimmed.upload, 50);
    }

    #[test]
    fn airmax_ptmp_ap_uses_directional_capacities_when_total_missing() {
        let device: Device = serde_json::from_value(json!({
            "identification": {
                "id": "airmax-ap-2",
                "hostname": "airmax-ap-2",
                "model": "RP-5AC-Gen2",
                "role": "ap",
                "type": "airMax"
            },
            "overview": {
                "wirelessMode": "ap-ptmp",
                "downlinkCapacity": 220000000,
                "uplinkCapacity": 180000000,
                "theoreticalTotalCapacity": 1000000,
                "wirelessActiveInterfaceIds": ["main"]
            },
            "interfaces": [
                {
                    "identification": { "name": "main", "type": "wlan" },
                    "wireless": { "dlRatio": null }
                }
            ]
        }))
        .expect("device JSON must deserialize");

        let trimmed = UispDevice::from_uisp(&device, &test_config(), &test_ip_ranges(), &[]);
        assert_eq!(trimmed.download, 176);
        assert_eq!(trimmed.upload, 44);
    }

    #[test]
    fn airmax_ptp_ap_ignores_theoretical_total_capacity() {
        let device: Device = serde_json::from_value(json!({
            "identification": {
                "id": "airmax-ap-ptp-1",
                "hostname": "airmax-ap-ptp-1",
                "model": "GBE",
                "role": "ap",
                "type": "airMax"
            },
            "overview": {
                "wirelessMode": "ap-ptp",
                "downlinkCapacity": 1617000000,
                "uplinkCapacity": 1078000000,
                "theoreticalTotalCapacity": 19250000,
                "wirelessActiveInterfaceIds": ["main"]
            },
            "interfaces": [
                {
                    "identification": { "name": "main", "type": "wlan" },
                    "wireless": { "dlRatio": 90.0 }
                }
            ]
        }))
        .expect("device JSON must deserialize");

        let trimmed = UispDevice::from_uisp(&device, &test_config(), &test_ip_ranges(), &[]);
        assert_eq!(trimmed.download, 1617);
        assert_eq!(trimmed.upload, 1078);
    }

    #[test]
    fn non_airmax_ap_devices_keep_existing_capacity_path() {
        let mut config = test_config();
        config.uisp_integration.airmax_capacity = 0.5;

        let device: Device = serde_json::from_value(json!({
            "identification": {
                "id": "airmax-station-1",
                "hostname": "airmax-station-1",
                "model": "LBE-5AC-Gen2",
                "role": "station",
                "type": "airMax"
            },
            "overview": {
                "wirelessMode": "sta-ptmp",
                "downlinkCapacity": 300000000,
                "uplinkCapacity": 200000000,
                "theoreticalTotalCapacity": 200000000,
                "wirelessActiveInterfaceIds": ["main"]
            },
            "interfaces": [
                {
                    "identification": { "name": "main", "type": "wlan" },
                    "wireless": { "dlRatio": 90.0 }
                }
            ]
        }))
        .expect("device JSON must deserialize");

        let trimmed = UispDevice::from_uisp(&device, &config, &test_ip_ranges(), &[]);
        assert_eq!(trimmed.download, 150);
        assert_eq!(trimmed.upload, 100);
    }

    #[test]
    fn probe_ip_candidate_strips_cidr_and_rejects_bad_inputs() {
        assert_eq!(
            UispDevice::probe_ip_candidate("100.126.0.226/29").as_deref(),
            Some("100.126.0.226")
        );
        assert_eq!(
            UispDevice::probe_ip_candidate("2602:fdca::10/64").as_deref(),
            Some("2602:fdca::10")
        );
        assert_eq!(UispDevice::probe_ip_candidate("100.126.0.224/29"), None);
        assert_eq!(UispDevice::probe_ip_candidate("fe80::1/64"), None);
    }

    #[test]
    fn from_uisp_keeps_unfiltered_probe_ips_for_management_links() {
        let mut config = test_config();
        config.ip_ranges.allow_subnets = vec!["100.124.0.0/16".to_string()];
        config.ip_ranges.ignore_subnets.clear();

        let device: Device = serde_json::from_value(json!({
            "identification": {
                "id": "wave-probe-test",
                "hostname": "wave-probe-test",
                "model": "Wave-Pro",
                "role": "station",
                "type": "airFiber"
            },
            "ipAddress": "100.126.0.226/29",
            "interfaces": [
                {
                    "identification": { "name": "br0", "type": "bridge" },
                    "addresses": [
                        { "cidr": "100.126.0.226/29", "version": "v4", "type": "dynamic", "origin": "dhcp" },
                        { "cidr": "fe80::eea:14ff:fe4f:e609/64", "version": "v6", "type": "dynamic", "origin": "link-local" }
                    ]
                }
            ]
        }))
        .expect("device JSON must deserialize");

        let ip_ranges = IpRanges::new(&config).expect("test config should build ip ranges");
        let trimmed = UispDevice::from_uisp(&device, &config, &ip_ranges, &[]);

        assert_eq!(trimmed.ipv4, HashSet::new());
        assert!(trimmed.ipv6.is_empty());
        assert_eq!(
            trimmed.probe_ipv4,
            HashSet::from(["100.126.0.226".to_string()])
        );
        assert!(trimmed.probe_ipv6.is_empty());
    }

    #[test]
    fn af60lr_model_fallback_caps_dynamic_radio_capacity_to_gigabit_transport() {
        let device: Device = serde_json::from_value(json!({
            "identification": {
                "id": "af60lr-1",
                "hostname": "AF60LR-TuscToMonte",
                "model": "AF60-LR",
                "role": "station",
                "type": "airFiber"
            },
            "overview": {
                "wirelessMode": "sta-ptp",
                "downlinkCapacity": 2000000000i64,
                "uplinkCapacity": 2000000000i64
            }
        }))
        .expect("device JSON must deserialize");

        let trimmed = UispDevice::from_uisp(&device, &test_config(), &test_ip_ranges(), &[]);
        assert_eq!(trimmed.raw_download, 2000);
        assert_eq!(trimmed.raw_upload, 2000);
        assert_eq!(trimmed.download, 940);
        assert_eq!(trimmed.upload, 940);
        assert_eq!(trimmed.transport_cap_mbps, Some(940));
        assert!(
            trimmed
                .transport_cap_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("AF60LR"))
        );
    }

    #[test]
    fn wavepro_active_two_point_five_gig_port_caps_dynamic_radio_capacity() {
        let device: Device = serde_json::from_value(json!({
            "identification": {
                "id": "wavepro-1",
                "hostname": "WavePro-WestReddToRochester",
                "model": "Wave-Pro",
                "role": "station",
                "type": "airFiber"
            },
            "overview": {
                "wirelessMode": "sta-ptp",
                "downlinkCapacity": 2700000000i64,
                "uplinkCapacity": 2700000000i64
            },
            "interfaces": [
                {
                    "identification": { "name": "eth0", "type": "eth" },
                    "status": { "status": "connected", "speed": "auto", "currentSpeed": "2.5Gbps-full" },
                    "wireless": {}
                }
            ]
        }))
        .expect("device JSON must deserialize");

        let trimmed = UispDevice::from_uisp(&device, &test_config(), &test_ip_ranges(), &[]);
        assert_eq!(trimmed.raw_download, 2700);
        assert_eq!(trimmed.raw_upload, 2700);
        assert_eq!(trimmed.download, 2350);
        assert_eq!(trimmed.upload, 2350);
        assert_eq!(trimmed.transport_cap_mbps, Some(2350));
        assert!(
            trimmed
                .transport_cap_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("2500 Mbps"))
        );
    }

    #[test]
    fn mixed_active_transport_interfaces_use_highest_observed_speed() {
        let device: Device = serde_json::from_value(json!({
            "identification": {
                "id": "wavepro-ambiguous",
                "hostname": "WavePro-Ambiguous",
                "model": "Wave-Pro",
                "role": "station",
                "type": "airFiber"
            },
            "overview": {
                "wirelessMode": "sta-ptp",
                "downlinkCapacity": 2700000000i64,
                "uplinkCapacity": 2700000000i64
            },
            "interfaces": [
                {
                    "identification": { "name": "eth0", "type": "eth" },
                    "status": { "status": "connected", "speed": "auto", "currentSpeed": "1Gbps-full" },
                    "wireless": {}
                },
                {
                    "identification": { "name": "eth1", "type": "eth" },
                    "status": { "status": "connected", "speed": "auto", "currentSpeed": "2.5Gbps-full" },
                    "wireless": {}
                }
            ]
        }))
        .expect("device JSON must deserialize");

        let trimmed = UispDevice::from_uisp(&device, &test_config(), &test_ip_ranges(), &[]);
        assert_eq!(trimmed.raw_download, 2700);
        assert_eq!(trimmed.download, 2350);
        assert_eq!(trimmed.upload, 2350);
        assert_eq!(trimmed.transport_cap_mbps, Some(2350));
        assert!(
            trimmed
                .transport_cap_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("eth1") && reason.contains("2500 Mbps"))
        );
    }

    #[test]
    fn bridge_interfaces_do_not_override_faster_transport_ports() {
        let device: Device = serde_json::from_value(json!({
            "identification": {
                "id": "wavepro-bridge",
                "hostname": "WavePro-Bridge",
                "model": "Wave-Pro",
                "role": "station",
                "type": "airFiber"
            },
            "overview": {
                "wirelessMode": "sta-ptp",
                "downlinkCapacity": 2700000000i64,
                "uplinkCapacity": 2700000000i64
            },
            "interfaces": [
                {
                    "identification": { "name": "br0", "type": "bridge" },
                    "status": { "status": "connected", "speed": "auto", "currentSpeed": "1Gbps-full" },
                    "wireless": {}
                },
                {
                    "identification": { "name": "sfp0", "type": "sfp" },
                    "status": { "status": "connected", "speed": "auto", "currentSpeed": "2.5Gbps-full" },
                    "wireless": {}
                }
            ]
        }))
        .expect("device JSON must deserialize");

        let trimmed = UispDevice::from_uisp(&device, &test_config(), &test_ip_ranges(), &[]);
        assert_eq!(trimmed.raw_download, 2700);
        assert_eq!(trimmed.download, 2350);
        assert_eq!(trimmed.upload, 2350);
        assert_eq!(trimmed.transport_cap_mbps, Some(2350));
        assert!(
            trimmed
                .transport_cap_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("sfp0") && reason.contains("2500 Mbps"))
        );
    }
}
