use lqos_bus::{IpStats, TcHandle};
use lqos_config::ShapedDevice;
use lqos_network_devices::NetworkDevicesCatalog;
use lqos_utils::XdpIpAddress;
use lqos_utils::units::{DownUpOrder, TcpRetransmitSample};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

fn truncate_by_chars(input: &str, max_chars: usize) -> String {
    input.chars().take(max_chars).collect()
}

fn shaped_device_for_ip_stats<'a>(
    stat: &IpStats,
    catalog: &'a NetworkDevicesCatalog,
) -> Option<&'a ShapedDevice> {
    if !stat.circuit_id.is_empty()
        && let Some(circuit) = catalog
            .iter_all_devices()
            .find(|sd| sd.circuit_id == stat.circuit_id)
    {
        return Some(circuit);
    }

    let ip = stat.ip_address.parse::<IpAddr>().ok()?;
    catalog
        .device_longest_match_for_ip(&XdpIpAddress::from_ip(ip))
        .map(|(_net, device)| device)
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct IpStatsWithPlan {
    pub ip_address: String,
    pub bits_per_second: DownUpOrder<u64>,
    pub packets_per_second: DownUpOrder<u64>,
    pub median_tcp_rtt: f32,
    pub tc_handle: TcHandle,
    pub circuit_id: String,
    pub plan: DownUpOrder<f32>,
    pub tcp_retransmit_sample: DownUpOrder<TcpRetransmitSample>,
}

impl From<&IpStats> for IpStatsWithPlan {
    fn from(i: &IpStats) -> Self {
        let mut result = Self {
            ip_address: i.ip_address.clone(),
            bits_per_second: i.bits_per_second,
            packets_per_second: i.packets_per_second,
            median_tcp_rtt: i.median_tcp_rtt,
            tc_handle: i.tc_handle,
            circuit_id: i.circuit_id.clone(),
            plan: DownUpOrder { down: 0.0, up: 0.0 },
            tcp_retransmit_sample: i.tcp_retransmit_sample,
        };

        let catalog = lqos_network_devices::network_devices_catalog();
        if let Some(circuit) = shaped_device_for_ip_stats(i, &catalog) {
            if result.circuit_id.is_empty() {
                result.circuit_id = circuit.circuit_id.clone();
            }
            let name = if circuit.circuit_name.chars().count() > 20 {
                truncate_by_chars(&circuit.circuit_name, 20)
            } else {
                circuit.circuit_name.clone()
            };
            result.ip_address = name.to_string();
            result.plan = DownUpOrder {
                down: circuit.download_max_mbps,
                up: circuit.upload_max_mbps,
            };
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::{shaped_device_for_ip_stats, truncate_by_chars};
    use lqos_bus::{IpStats, TcHandle};
    use lqos_config::{ConfigShapedDevices, ShapedDevice};
    use lqos_utils::units::{DownUpOrder, TcpRetransmitSample};
    use std::net::Ipv4Addr;
    use std::sync::Arc;

    #[test]
    fn truncates_ascii_to_exact_length() {
        assert_eq!(
            truncate_by_chars("abcdefghijklmnopqrstuvwxyz", 20),
            "abcdefghijklmnopqrst"
        );
    }

    #[test]
    fn truncates_utf8_without_panicking_on_char_boundaries() {
        assert_eq!(
            truncate_by_chars("Punčochářová, Věra", 15),
            "Punčochářová, V"
        );
    }

    #[test]
    fn keeps_short_strings_unchanged() {
        assert_eq!(truncate_by_chars("Věra", 20), "Věra");
    }

    #[test]
    fn shaped_device_lookup_falls_back_to_ip_when_circuit_id_is_blank() {
        let mut shaped = ConfigShapedDevices::default();
        shaped.replace_with_new_data(vec![ShapedDevice {
            circuit_id: "circuit-1".to_string(),
            circuit_name: "Circuit Alpha".to_string(),
            device_id: "device-1".to_string(),
            parent_node: "Parent-A".to_string(),
            ipv4: vec![(Ipv4Addr::new(192, 168, 1, 10), 32)],
            ..Default::default()
        }]);

        let shaped_catalog =
            lqos_network_devices::ShapedDevicesCatalog::from_shaped_devices(Arc::new(shaped));
        let catalog = lqos_network_devices::NetworkDevicesCatalog::from_snapshots(
            shaped_catalog,
            Arc::new(Vec::new()),
        );

        let stat = IpStats {
            ip_address: "192.168.1.10".to_string(),
            circuit_id: String::new(),
            bits_per_second: DownUpOrder::zeroed(),
            packets_per_second: DownUpOrder::zeroed(),
            median_tcp_rtt: 0.0,
            tc_handle: TcHandle::from_u32(0),
            tcp_retransmit_sample: DownUpOrder::new(
                TcpRetransmitSample::new(0, 0),
                TcpRetransmitSample::new(0, 0),
            ),
        };

        let matched = shaped_device_for_ip_stats(&stat, &catalog).expect("lookup should resolve");
        assert_eq!(matched.circuit_id, "circuit-1");
    }
}
