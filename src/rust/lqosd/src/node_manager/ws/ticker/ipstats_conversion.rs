use crate::shaped_devices_tracker::SHAPED_DEVICES;
use lqos_bus::{IpStats, TcHandle};
use lqos_utils::units::DownUpOrder;
use serde::{Deserialize, Serialize};

fn truncate_by_chars(input: &str, max_chars: usize) -> String {
    input.chars().take(max_chars).collect()
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
    pub tcp_retransmits: (f64, f64),
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
            tcp_retransmits: i.tcp_retransmits,
        };

        if !result.circuit_id.is_empty() {
            let shaped_devices_reader = SHAPED_DEVICES.load();
            if let Some(circuit) = shaped_devices_reader
                .devices
                .iter()
                .find(|sd| sd.circuit_id == result.circuit_id)
            {
                let name = if circuit.circuit_name.chars().count() > 20 {
                    truncate_by_chars(&circuit.circuit_name, 20)
                } else {
                    circuit.circuit_name.clone()
                };
                result.ip_address = format!("{}", name);
                result.plan = DownUpOrder {
                    down: circuit.download_max_mbps as f32,
                    up: circuit.upload_max_mbps as f32,
                };
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::truncate_by_chars;

    #[test]
    fn truncates_ascii_to_exact_length() {
        assert_eq!(truncate_by_chars("abcdefghijklmnopqrstuvwxyz", 20), "abcdefghijklmnopqrst");
    }

    #[test]
    fn truncates_utf8_without_panicking_on_char_boundaries() {
        assert_eq!(truncate_by_chars("Punčochářová, Věra", 15), "Punčochářová, V");
    }

    #[test]
    fn keeps_short_strings_unchanged() {
        assert_eq!(truncate_by_chars("Věra", 20), "Věra");
    }
}
