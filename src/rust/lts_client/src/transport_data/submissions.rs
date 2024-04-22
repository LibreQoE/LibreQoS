//! Holds data-types to be submitted as part of long-term stats
//! collection.

use lqos_config::ShapedDevice;
use serde::{Deserialize, Serialize};
use uisp::Device;

use crate::collector::CakeStats;

/// Type that provides a minimum, maximum and average value
/// for a given statistic within the associated time period.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StatsSummary {
    /// Minimum value
    pub min: (u64, u64),
    /// Maximum value
    pub max: (u64, u64),
    /// Average value
    pub avg: (u64, u64),
}

/// Type that provides a minimum, maximum and average value
/// for a given RTT value within the associated time period.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StatsRttSummary {
    /// Minimum value
    pub min: u32,
    /// Maximum value
    pub max: u32,
    /// Average value
    pub avg: u32,
}

/// Type that holds total traffic statistics for a given time period
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StatsTotals {
    /// Total number of packets
    pub packets: StatsSummary,
    /// Total number of bits
    pub bits: StatsSummary,
    /// Total number of shaped bits
    pub shaped_bits: StatsSummary,
}

/// Type that holds per-host statistics for a given stats collation
/// period.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StatsHost {
    /// Host circuit_id as it appears in ShapedDevices.csv
    pub circuit_id: Option<String>,
    /// Host's IP address
    pub ip_address: String,
    /// Host's traffic statistics
    pub bits: StatsSummary,
    /// Host's RTT statistics
    pub rtt: StatsRttSummary,
}

/// Node inside a traffic summary tree
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StatsTreeNode {
    /// Index in the tree vector
    pub index: usize,
    /// Name (from network.json)
    pub name: String,
    /// Maximum allowed throughput (from network.json)
    pub max_throughput: (u32, u32),
    /// Current throughput (from network.json)
    pub current_throughput: StatsSummary,
    /// RTT summaries
    pub rtt: StatsRttSummary,
    /// Indices of parents in the tree
    pub parents: Vec<usize>,
    /// Index of immediate parent in the tree
    pub immediate_parent: Option<usize>,
    /// Node Type
    pub node_type: Option<String>,
}

/// Collation of all stats for a given time period
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StatsSubmission {
    /// Timestamp of the collation (UNIX time)
    pub timestamp: u64,
    /// Total traffic statistics
    pub totals: Option<StatsTotals>,
    /// Per-host statistics
    pub hosts: Option<Vec<StatsHost>>,
    /// Tree of traffic summaries
    pub tree: Option<Vec<StatsTreeNode>>,
    /// CPU utiliation on the shaper
    pub cpu_usage: Option<Vec<u32>>,
    /// RAM utilization on the shaper
    pub ram_percent: Option<u32>,
    /// UISP Device Information
    pub uisp_devices: Option<Vec<UispExtDevice>>,
    /// Queue Stats
    pub cake_stats: Option<(Vec<CakeStats>, Vec<CakeStats>)>,

}

/// Submission to the `lts_node` process
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum LtsCommand {
    Submit(Box<StatsSubmission>),
    Devices(Vec<ShapedDevice>),
}

/// Extended data provided from UISP
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct UispExtDevice {
    pub device_id: String,
    pub name: String,
    pub model: String,
    pub firmware: String,
    pub status: String,
    pub frequency: f64,
    pub channel_width: i32,
    pub tx_power: i32,
    pub rx_signal: i32,
    pub downlink_capacity_mbps: i32,
    pub uplink_capacity_mbps: i32,
    pub noise_floor: i32,
    pub mode: String,
    pub interfaces: Vec<UispExtDeviceInterface>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct UispExtDeviceInterface {
    pub name: String,
    pub mac: String,
    pub ip: Vec<String>,
    pub status: String,
    pub speed: String,
}

impl From<Device> for UispExtDevice {
    fn from(d: Device) -> Self {
        let device_id = d.identification.id.to_string();
        let device_name = d.get_name().as_ref().unwrap_or(&"".to_string()).to_string();
        let model = d
            .identification
            .modelName
            .as_ref()
            .unwrap_or(&"".to_string())
            .to_string();
        let firmware = d
            .identification
            .firmwareVersion
            .as_ref()
            .unwrap_or(&"".to_string())
            .to_string();
        let mode = d.mode.as_ref().unwrap_or(&"".to_string()).to_string();
        let status;
        let frequency;
        let channel_width;
        let tx_power;
        let rx_signal;
        let downlink_capacity_mbps;
        let uplink_capacity_mbps;
        if let Some(ov) = &d.overview {
            status = ov.status.as_ref().unwrap_or(&"".to_string()).to_string();
            frequency = ov.frequency.unwrap_or(0.0);
            channel_width = ov.channelWidth.unwrap_or(0);
            tx_power = ov.transmitPower.unwrap_or(0);
            rx_signal = ov.signal.unwrap_or(0);
            downlink_capacity_mbps = ov.downlinkCapacity.unwrap_or(0);
            uplink_capacity_mbps = ov.uplinkCapacity.unwrap_or(0);
        } else {
            status = "".to_string();
            frequency = 0.0;
            channel_width = 0;
            tx_power = 0;
            rx_signal = 0;
            downlink_capacity_mbps = 0;
            uplink_capacity_mbps = 0;
        }

        let mut noise_floor = 0;
        let mut iflist = Vec::new();
        if let Some(interfaces) = &d.interfaces {
            interfaces.iter().for_each(|i| {
                if let Some(wireless) = &i.wireless {
                    if let Some(nf) = wireless.noiseFloor {
                        noise_floor = nf;
                    }
                }

                if let Some(addr) = &i.addresses {
                    let mut ip = Vec::new();
                    addr.iter().for_each(|a| {
                        if let Some(ipaddr) = &a.cidr {
                            ip.push(ipaddr.to_string());
                        }
                    });
                }

                let mut if_name = "".to_string();
                let mut if_mac = "".to_string();
                if let Some(id) = &i.identification {
                    if let Some(name) = &id.name {
                        if_name = name.to_string();
                    }
                    if let Some(mac) = &id.mac {
                        if_mac = mac.to_string();
                    }
                }

                let mut if_status = "".to_string();
                let mut if_speed = "".to_string();
                if let Some(status) = &i.status {
                    if let Some(s) = &status.status {
                        if_status = s.to_string();
                    }
                    if let Some(s) = &status.speed {
                        if_speed = s.to_string();
                    }
                }

                let mut if_ip = Vec::new();
                if let Some(addr) = &i.addresses {
                    addr.iter().for_each(|a| {
                        if let Some(ipaddr) = &a.cidr {
                            if_ip.push(ipaddr.to_string());
                        }
                    });
                }

                iflist.push(UispExtDeviceInterface {
                    name: if_name,
                    mac: if_mac,
                    status: if_status,
                    speed: if_speed,
                    ip: if_ip,
                });
            });
        }

        Self {
            device_id,
            name: device_name,
            model,
            firmware,
            status,
            frequency,
            channel_width,
            tx_power,
            rx_signal,
            downlink_capacity_mbps: downlink_capacity_mbps as i32,
            uplink_capacity_mbps: uplink_capacity_mbps as i32,
            noise_floor,
            mode,
            interfaces: iflist,
        }
    }
}
