use lqos_bus::{Circuit, CircuitRollup, FlowbeeSummaryData, IpStats};
use lqos_utils::units::{DownUpOrder, TcpRetransmitSample};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
pub struct DownUpTransit {
    pub down: u64,
    pub up: u64,
}

impl From<DownUpOrder<u64>> for DownUpTransit {
    fn from(value: DownUpOrder<u64>) -> Self {
        Self {
            down: value.down,
            up: value.up,
        }
    }
}

impl From<DownUpOrder<f32>> for DownUpTransit {
    fn from(value: DownUpOrder<f32>) -> Self {
        Self {
            down: value.down as u64,
            up: value.up as u64,
        }
    }
}

impl From<DownUpOrder<u32>> for DownUpTransit {
    fn from(value: DownUpOrder<u32>) -> Self {
        Self {
            down: value.down as u64,
            up: value.up as u64,
        }
    }
}

impl From<DownUpOrder<u16>> for DownUpTransit {
    fn from(value: DownUpOrder<u16>) -> Self {
        Self {
            down: value.down as u64,
            up: value.up as u64,
        }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
pub struct DownUpF32Transit {
    pub down: f32,
    pub up: f32,
}

impl From<DownUpOrder<f32>> for DownUpF32Transit {
    fn from(value: DownUpOrder<f32>) -> Self {
        Self {
            down: value.down,
            up: value.up,
        }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
pub struct DownUpOptionalU64Transit {
    pub down: Option<u64>,
    pub up: Option<u64>,
}

impl From<DownUpOrder<Option<u64>>> for DownUpOptionalU64Transit {
    fn from(value: DownUpOrder<Option<u64>>) -> Self {
        Self {
            down: value.down,
            up: value.up,
        }
    }
}

fn retransmits_from_sample(value: DownUpOrder<TcpRetransmitSample>) -> DownUpTransit {
    DownUpTransit {
        down: value.down.retransmits.get(),
        up: value.up.retransmits.get(),
    }
}

fn packets_from_sample(value: DownUpOrder<TcpRetransmitSample>) -> DownUpTransit {
    DownUpTransit {
        down: value.down.packets.get(),
        up: value.up.packets.get(),
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct TcHandleTransit {
    pub major: u16,
    pub minor: u16,
}

impl From<lqos_bus::TcHandle> for TcHandleTransit {
    fn from(value: lqos_bus::TcHandle) -> Self {
        let (major, minor) = value.get_major_minor();
        Self { major, minor }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct IpstatsTransit {
    /// The host's IP address, as detected by the XDP program.
    pub ip_address: String,

    /// The host's mapped circuit ID
    pub circuit_id: String,

    /// The host's mapped circuit name, if known
    pub circuit_name: Option<String>,

    /// The current bits-per-second passing through this host.
    pub bits_per_second: DownUpTransit,

    /// The current packets-per-second passing through this host. Tuple
    /// 0 is download, tuple 1 is upload.
    pub packets_per_second: DownUpTransit,

    /// Median TCP round-trip-time for this host at the current time.
    pub median_tcp_rtt: f32,

    /// Associated TC traffic control handle.
    pub tc_handle: TcHandleTransit,

    /// TCP Retransmits for this host at the current time.
    pub tcp_retransmits: (f64, f64),
}

impl From<IpStats> for IpstatsTransit {
    fn from(value: IpStats) -> Self {
        Self {
            ip_address: value.ip_address,
            circuit_id: value.circuit_id,
            circuit_name: None,
            bits_per_second: DownUpTransit::from(value.bits_per_second),
            packets_per_second: DownUpTransit::from(value.packets_per_second),
            median_tcp_rtt: value.median_tcp_rtt,
            tc_handle: TcHandleTransit::from(value.tc_handle),
            tcp_retransmits: (
                value.tcp_retransmit_sample.down.retransmits.get() as f64,
                value.tcp_retransmit_sample.up.retransmits.get() as f64,
            ),
        }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct CircuitTransit {
    /// The IP address of the host.
    pub ip: String,
    /// Current bytes-per-second passing through this host.
    pub bytes_per_second: DownUpTransit,
    /// Current bytes-per-second actually transmitted for this host.
    pub actual_bytes_per_second: DownUpTransit,
    /// Median latency for this host at the current time.
    pub median_latency: Option<f32>,
    /// TCP Retransmits for this host at the current time.
    pub tcp_retransmits: DownUpTransit,
    /// The number of TCP packets per second.
    pub tcp_packets: DownUpTransit,
    /// The mapped circuit ID
    pub circuit_id: Option<String>,
    /// The mapped device ID
    pub device_id: Option<String>,
    /// The parent node of the device
    pub parent_node: Option<String>,
    /// The circuit name
    pub circuit_name: Option<String>,
    /// The device name
    pub device_name: Option<String>,
    /// The current plan for this circuit.
    pub plan: DownUpTransit,
    /// Age of this row's last activity, in nanoseconds since the circuit was
    /// last seen. `null` = no activity age is reported for this row (the
    /// node has no live sample for it), never "seen just now".
    pub last_seen_nanos: Option<u64>,
    /// Current RTT p50 per direction, in nanoseconds. `null` per direction =
    /// no RTT sample for this circuit; v3 never fills an unmeasured leg with 0.
    pub rtt_current_p50_nanos: DownUpOptionalU64Transit,
}

impl CircuitTransit {
    /// Overrides the circuit-level activity age with the daemon's live rollup.
    pub fn with_activity_age(mut self, age: Option<u64>) -> Self {
        self.last_seen_nanos = age;
        self
    }

    /// Overrides the RTT legs with the daemon's live rollup values.
    pub fn with_rtt_current_p50(mut self, rtt: DownUpOptionalU64Transit) -> Self {
        self.rtt_current_p50_nanos = rtt;
        self
    }
}

impl From<Circuit> for CircuitTransit {
    fn from(value: Circuit) -> Self {
        Self {
            ip: value.ip.to_string(),
            bytes_per_second: DownUpTransit::from(value.bytes_per_second),
            actual_bytes_per_second: DownUpTransit::from(value.actual_bytes_per_second),
            median_latency: value.median_latency,
            tcp_retransmits: retransmits_from_sample(value.tcp_retransmit_sample),
            tcp_packets: packets_from_sample(value.tcp_retransmit_sample),
            circuit_id: value.circuit_id,
            device_id: value.device_id,
            parent_node: value.parent_node,
            circuit_name: value.circuit_name,
            device_name: value.device_name,
            plan: DownUpTransit::from(value.plan),
            // The bus row's zero is "no age reported", not "seen now".
            last_seen_nanos: (value.last_seen_nanos != 0).then_some(value.last_seen_nanos),
            rtt_current_p50_nanos: DownUpOptionalU64Transit::from(value.rtt_current_p50_nanos),
        }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
pub struct CircuitRollupTransit {
    /// Circuit ID from ShapedDevices.csv.
    pub circuit_id: String,
    /// Circuit name from ShapedDevices.csv.
    pub circuit_name: String,
    /// Effective parent node for this circuit.
    pub parent_node: String,
    /// Device names contributing to this circuit rollup.
    pub device_names: Vec<String>,
    /// Active IP addresses contributing to this circuit rollup.
    pub ip_addrs: Vec<String>,
    /// Circuit plan in Mbps.
    pub plan_mbps: DownUpF32Transit,
    /// Current bytes-per-second passing through this circuit.
    pub bytes_per_second: DownUpTransit,
    /// Current RTT p50 in nanoseconds, per direction.
    pub rtt_current_p50_nanos: DownUpOptionalU64Transit,
    /// Current QoO score, per direction.
    pub qoo: QooLatestTransit,
    /// Current TCP retransmit counts, per direction.
    pub tcp_retransmits: DownUpTransit,
    /// Current TCP packet counts, per direction.
    pub tcp_packets: DownUpTransit,
    /// Most recent activity age for this circuit, in nanoseconds since boot.
    pub last_seen_nanos: u64,
}

impl From<CircuitRollup> for CircuitRollupTransit {
    fn from(value: CircuitRollup) -> Self {
        Self {
            circuit_id: value.circuit_id,
            circuit_name: value.circuit_name,
            parent_node: value.parent_node,
            device_names: value.device_names,
            ip_addrs: value.ip_addrs,
            plan_mbps: DownUpF32Transit::from(value.plan_mbps),
            bytes_per_second: DownUpTransit::from(value.bytes_per_second),
            rtt_current_p50_nanos: DownUpOptionalU64Transit::from(value.rtt_current_p50_nanos),
            qoo: QooLatestTransit::from(value.qoo),
            tcp_retransmits: retransmits_from_sample(value.tcp_retransmit_sample),
            tcp_packets: packets_from_sample(value.tcp_retransmit_sample),
            last_seen_nanos: value.last_seen_nanos,
        }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct FlowbeeSummaryDataTransit {
    /// Mapped `XdpIpAddress` source for the flow.
    pub remote_ip: String,
    /// Mapped `XdpIpAddress` destination for the flow
    pub local_ip: String,
    /// Source port number, or ICMP type.
    pub src_port: u16,
    /// Destination port number.
    pub dst_port: u16,
    /// IP protocol (see the Linux kernel!)
    pub ip_protocol: FlowbeeProtocolTransit,
    /// Padding to align the structure to 16 bytes.
    /// Time (nanos) when the connection was established
    pub start_time: u64,
    /// Time (nanos) when the connection was last seen
    pub last_seen: u64,
    /// Bytes transmitted
    pub bytes_sent: DownUpTransit,
    /// Packets transmitted
    pub packets_sent: DownUpTransit,
    /// Rate estimate
    pub rate_estimate_bps: DownUpTransit,
    /// TCP Retransmission count (also counts duplicates)
    pub tcp_retransmits: DownUpTransit,
    /// Has the connection ended?
    /// 0 = Alive, 1 = FIN, 2 = RST
    pub end_status: u8,
    /// Raw IP TOS
    pub tos: u8,
    /// Raw TCP flags
    pub flags: u8,
    /// Recent RTT median
    pub rtt_nanos: DownUpTransit,
    /// Remote ASN
    pub remote_asn: u32,
    /// Remote ASN Name
    pub remote_asn_name: String,
    /// Remote ASN Country
    pub remote_asn_country: String,
    /// Analysis
    pub analysis: String,
    /// Circuit ID
    pub circuit_id: String,
    /// Circuit Name
    pub circuit_name: String,
}

impl From<FlowbeeSummaryData> for FlowbeeSummaryDataTransit {
    fn from(_value: FlowbeeSummaryData) -> Self {
        Self {
            remote_ip: _value.remote_ip,
            local_ip: _value.local_ip,
            src_port: _value.src_port,
            dst_port: _value.dst_port,
            ip_protocol: FlowbeeProtocolTransit::from(_value.ip_protocol),
            start_time: _value.start_time,
            last_seen: _value.last_seen,
            bytes_sent: DownUpTransit::from(_value.bytes_sent),
            packets_sent: DownUpTransit::from(_value.packets_sent),
            rate_estimate_bps: DownUpTransit::from(_value.rate_estimate_bps),
            tcp_retransmits: DownUpTransit::from(_value.tcp_retransmits),
            end_status: _value.end_status,
            tos: _value.tos,
            flags: _value.flags,
            rtt_nanos: DownUpTransit::from(_value.rtt_nanos),
            remote_asn: _value.remote_asn,
            remote_asn_name: _value.remote_asn_name,
            remote_asn_country: _value.remote_asn_country,
            analysis: _value.analysis,
            circuit_id: _value.circuit_id,
            circuit_name: _value.circuit_name,
        }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub enum FlowbeeProtocolTransit {
    /// TCP (type 6)
    TCP,
    /// UDP (type 17)
    UDP,
    /// ICMP (type 1)
    ICMP,
}

impl From<lqos_bus::FlowbeeProtocol> for FlowbeeProtocolTransit {
    fn from(value: lqos_bus::FlowbeeProtocol) -> Self {
        match value {
            lqos_bus::FlowbeeProtocol::TCP => Self::TCP,
            lqos_bus::FlowbeeProtocol::UDP => Self::UDP,
            lqos_bus::FlowbeeProtocol::ICMP => Self::ICMP,
        }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
pub struct QooLatestTransit {
    /// Latest download QoO score.
    pub down: Option<f32>,
    /// Latest upload QoO score.
    pub up: Option<f32>,
}

impl From<lqos_utils::units::DownUpOrder<Option<f32>>> for QooLatestTransit {
    fn from(value: lqos_utils::units::DownUpOrder<Option<f32>>) -> Self {
        Self {
            down: value.down,
            up: value.up,
        }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
pub struct QooBlocksTransit {
    /// Fifteen download QoO history blocks, oldest to newest.
    pub download_total: Vec<Option<f32>>,
    /// Fifteen upload QoO history blocks, oldest to newest.
    pub upload_total: Vec<Option<f32>>,
}

impl From<lqos_utils::qoq_heatmap::QoqHeatmapBlocks> for QooBlocksTransit {
    fn from(value: lqos_utils::qoq_heatmap::QoqHeatmapBlocks) -> Self {
        Self {
            download_total: value.download_total.to_vec(),
            upload_total: value.upload_total.to_vec(),
        }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
pub struct QooDataTransit {
    /// Stable QoO row key.
    pub key: String,
    /// Entity kind: "global", "site", or "circuit".
    pub entity_kind: String,
    /// Display label for the QoO row.
    pub label: String,
    /// Site name for site rows.
    pub site_name: Option<String>,
    /// Circuit ID for circuit rows.
    pub circuit_id: Option<String>,
    /// Fifteen QoO history blocks.
    pub blocks: QooBlocksTransit,
    /// Latest download and upload QoO scores.
    pub latest: QooLatestTransit,
}

impl From<lqos_bus::QooData> for QooDataTransit {
    fn from(value: lqos_bus::QooData) -> Self {
        Self {
            key: value.key,
            entity_kind: value.entity_kind,
            label: value.label,
            site_name: value.site_name,
            circuit_id: value.circuit_id,
            blocks: QooBlocksTransit::from(value.blocks),
            latest: QooLatestTransit::from(value.latest),
        }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct NetworkJsonTransit {
    /// Display name
    pub name: String,
    /// Max throughput for node (not clamped), in Mbps (download, upload).
    /// Values may be fractional.
    pub max_permitted_throughput_mbps: (f64, f64),
    /// Current node throughput
    pub current_throughput_bits_per_second: (u64, u64),
    /// Current node packets
    pub current_packets_per_second: (u64, u64),
    /// Current TCP packets
    pub current_tcp_packets_per_second: (u64, u64),
    /// Current UDP packets
    pub current_udp_packets_per_second: (u64, u64),
    /// Current ICMP packets
    pub current_icmp_packets: (u64, u64),
    /// Current TCP retransmits as a percentage (0.0–100.0)
    /// Note: This is already scaled to percent. Do not divide again.
    pub current_retransmits_percent: Option<(f64, f64)>,
    /// Cake marks
    pub current_cake_ecn_marks_percent: (f64, f64),
    /// Cake drops
    pub current_cake_drops_percent: (f64, f64),
    /// Set of RTT data
    pub tcp_round_trip_time_ms_median: f32,
    pub tcp_round_trip_time_ms_95th: f32,
    /// Node indices of parents
    pub parents: Vec<usize>,
    /// The immediate parent node in the tree
    pub immediate_parent: Option<usize>,
    /// The type of node (site, ap, etc.)
    #[serde(rename = "type")]
    pub node_type: Option<String>,
}

impl From<lqos_config::NetworkJsonTransport> for NetworkJsonTransit {
    fn from(_value: lqos_config::NetworkJsonTransport) -> Self {
        let mut rtt_set: Vec<f32> = _value
            .rtts
            .iter()
            .copied()
            .filter(|sample| sample.is_finite())
            .collect();
        rtt_set.sort_by(f32::total_cmp);
        let median = if rtt_set.is_empty() {
            0.0
        } else {
            rtt_set[rtt_set.len() / 2]
        };
        let p95 = if rtt_set.is_empty() {
            0.0
        } else {
            rtt_set[((rtt_set.len() as f32) * 0.95).ceil() as usize - 1]
        };

        Self {
            name: _value.name,
            max_permitted_throughput_mbps: _value.max_throughput,
            current_throughput_bits_per_second: (
                _value.current_throughput.0 * 8,
                _value.current_throughput.1 * 8,
            ),
            current_packets_per_second: _value.current_packets,
            current_tcp_packets_per_second: _value.current_tcp_packets,
            current_udp_packets_per_second: _value.current_udp_packets,
            current_icmp_packets: _value.current_icmp_packets,
            current_retransmits_percent: {
                if _value.current_tcp_packets == (0, 0) {
                    None
                } else {
                    Some((
                        if _value.current_tcp_packets.0 == 0 {
                            0.0
                        } else {
                            (_value.current_retransmits.0 as f64
                                / _value.current_tcp_packets.0 as f64)
                                * 100.0
                        },
                        if _value.current_tcp_packets.1 == 0 {
                            0.0
                        } else {
                            (_value.current_retransmits.1 as f64
                                / _value.current_tcp_packets.1 as f64)
                                * 100.0
                        },
                    ))
                }
            },
            // Percent values (0.0–100.0). Already scaled; do not divide again downstream.
            current_cake_ecn_marks_percent: (
                if _value.current_packets.0 == 0 {
                    0.0
                } else {
                    (_value.current_marks.0 as f64 / _value.current_packets.0 as f64) * 100.0
                },
                if _value.current_packets.1 == 0 {
                    0.0
                } else {
                    (_value.current_marks.1 as f64 / _value.current_packets.1 as f64) * 100.0
                },
            ),
            current_cake_drops_percent: (
                if _value.current_packets.0 == 0 {
                    0.0
                } else {
                    (_value.current_drops.0 as f64 / _value.current_packets.0 as f64) * 100.0
                },
                if _value.current_packets.1 == 0 {
                    0.0
                } else {
                    (_value.current_drops.1 as f64 / _value.current_packets.1 as f64) * 100.0
                },
            ),
            tcp_round_trip_time_ms_median: median,
            tcp_round_trip_time_ms_95th: p95,
            parents: _value.parents,
            immediate_parent: _value.immediate_parent,
            node_type: _value.node_type,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn network_transport_with_rtts(rtts: Vec<f32>) -> lqos_config::NetworkJsonTransport {
        lqos_config::NetworkJsonTransport {
            name: "Test Node".to_string(),
            id: None,
            max_throughput: (0.0, 0.0),
            configured_max_throughput: (0.0, 0.0),
            effective_max_throughput: None,
            current_throughput: (0, 0),
            current_packets: (0, 0),
            current_tcp_packets: (0, 0),
            current_udp_packets: (0, 0),
            current_icmp_packets: (0, 0),
            current_retransmits: (0, 0),
            current_tcp_retransmit_packets: (0, 0),
            current_rtt_flows: (0, 0),
            current_marks: (0, 0),
            current_drops: (0, 0),
            rtts,
            qoo: (None, None),
            parents: Vec::new(),
            immediate_parent: None,
            node_type: None,
            latitude: None,
            longitude: None,
            active_attachment_name: None,
            subtree_site_count: 0,
            subtree_circuit_count: 0,
            subtree_device_count: 0,
        }
    }

    #[test]
    fn network_json_transit_ignores_nan_rtt_samples() {
        let transit = NetworkJsonTransit::from(network_transport_with_rtts(vec![10.0, f32::NAN]));

        assert_eq!(transit.tcp_round_trip_time_ms_median, 10.0);
        assert_eq!(transit.tcp_round_trip_time_ms_95th, 10.0);
    }

    #[test]
    fn network_json_transit_uses_zero_when_all_rtt_samples_are_non_finite() {
        let transit =
            NetworkJsonTransit::from(network_transport_with_rtts(vec![f32::NAN, f32::INFINITY]));

        assert_eq!(transit.tcp_round_trip_time_ms_median, 0.0);
        assert_eq!(transit.tcp_round_trip_time_ms_95th, 0.0);
    }

    #[test]
    fn circuit_transit_maps_enqueued_and_actual_throughput() {
        let circuit = CircuitTransit::from(lqos_bus::Circuit {
            ip: "192.0.2.1".parse().expect("valid test IP"),
            bytes_per_second: DownUpOrder {
                down: 12_345,
                up: 6_789,
            },
            actual_bytes_per_second: DownUpOrder {
                down: 11_111,
                up: 5_555,
            },
            median_latency: Some(12.5),
            rtt_current_p50_nanos: DownUpOrder::default(),
            rtt_current_p95_nanos: DownUpOrder::default(),
            rtt_total_p50_nanos: DownUpOrder::default(),
            rtt_total_p95_nanos: DownUpOrder::default(),
            qoo: DownUpOrder::default(),
            tcp_retransmit_sample: DownUpOrder::default(),
            circuit_id: Some("circuit-1".to_string()),
            device_id: Some("device-1".to_string()),
            parent_node: Some("site-1".to_string()),
            circuit_name: Some("Test Circuit".to_string()),
            device_name: Some("Test Device".to_string()),
            plan: DownUpOrder {
                down: 100.0,
                up: 20.0,
            },
            last_seen_nanos: 42,
        });

        assert_eq!(circuit.bytes_per_second.down, 12_345);
        assert_eq!(circuit.bytes_per_second.up, 6_789);
        assert_eq!(circuit.actual_bytes_per_second.down, 11_111);
        assert_eq!(circuit.actual_bytes_per_second.up, 5_555);

        let json = serde_json::to_value(circuit).expect("serialize circuit transit");
        assert_eq!(json["bytes_per_second"]["down"], 12_345);
        assert_eq!(json["actual_bytes_per_second"]["down"], 11_111);
    }

    fn bus_row(
        last_seen_nanos: u64,
        rtt_down: Option<u64>,
        rtt_up: Option<u64>,
    ) -> lqos_bus::Circuit {
        lqos_bus::Circuit {
            ip: "192.0.2.1".parse().expect("test ip"),
            bytes_per_second: DownUpOrder::zeroed(),
            actual_bytes_per_second: DownUpOrder::zeroed(),
            median_latency: None,
            rtt_current_p50_nanos: DownUpOrder {
                down: rtt_down,
                up: rtt_up,
            },
            rtt_current_p95_nanos: DownUpOrder::default(),
            rtt_total_p50_nanos: DownUpOrder::default(),
            rtt_total_p95_nanos: DownUpOrder::default(),
            qoo: DownUpOrder::default(),
            tcp_retransmit_sample: DownUpOrder::default(),
            circuit_id: Some("circuit-1".to_string()),
            device_id: Some("device-1".to_string()),
            parent_node: Some("site-1".to_string()),
            circuit_name: Some("Test Circuit".to_string()),
            device_name: Some("Test Device".to_string()),
            plan: DownUpOrder {
                down: 100.0,
                up: 20.0,
            },
            last_seen_nanos,
        }
    }

    /// A bus row's zero age/RTT is "not reported", never "seen now" or "0 ms".
    #[test]
    fn circuit_transit_reports_absent_activity_and_rtt_as_null() {
        let absent = CircuitTransit::from(bus_row(0, None, None));
        assert_eq!(absent.last_seen_nanos, None);
        assert_eq!(absent.rtt_current_p50_nanos.down, None);
        assert_eq!(absent.rtt_current_p50_nanos.up, None);

        let reported = CircuitTransit::from(bus_row(5, Some(1_000), None))
            .with_activity_age(Some(7))
            .with_rtt_current_p50(DownUpOptionalU64Transit {
                down: Some(2_000),
                up: None,
            });
        assert_eq!(reported.last_seen_nanos, Some(7));
        assert_eq!(reported.rtt_current_p50_nanos.down, Some(2_000));
        assert_eq!(reported.rtt_current_p50_nanos.up, None);
    }

    #[test]
    fn qoo_data_transit_maps_blocks_and_latest_values() {
        let mut download_total = [None; 15];
        let mut upload_total = [None; 15];
        download_total[14] = Some(92.0);
        upload_total[14] = Some(88.0);

        let qoo = QooDataTransit::from(lqos_bus::QooData {
            key: "site:North".to_string(),
            entity_kind: "site".to_string(),
            label: "North".to_string(),
            site_name: Some("North".to_string()),
            circuit_id: None,
            blocks: lqos_utils::qoq_heatmap::QoqHeatmapBlocks {
                download_total,
                upload_total,
            },
            latest: lqos_utils::units::DownUpOrder {
                down: Some(92.0),
                up: Some(88.0),
            },
        });

        assert_eq!(qoo.key, "site:North");
        assert_eq!(qoo.entity_kind, "site");
        assert_eq!(qoo.site_name.as_deref(), Some("North"));
        assert_eq!(qoo.circuit_id, None);
        assert_eq!(qoo.blocks.download_total.len(), 15);
        assert_eq!(qoo.blocks.upload_total.len(), 15);
        assert_eq!(qoo.latest.down, Some(92.0));
        assert_eq!(qoo.latest.up, Some(88.0));
    }

    #[test]
    fn circuit_rollup_transit_maps_combined_circuit_fields() {
        let rollup = CircuitRollupTransit::from(lqos_bus::CircuitRollup {
            circuit_id: "10001-10004".to_string(),
            circuit_name: "Crew Device Group".to_string(),
            parent_node: "Ship-10001-Crew".to_string(),
            device_names: vec!["Crew Devices".to_string()],
            ip_addrs: vec!["10.224.11.10".to_string(), "10.0.3.10".to_string()],
            plan_mbps: DownUpOrder {
                down: 10.0,
                up: 2.0,
            },
            bytes_per_second: DownUpOrder {
                down: 12345,
                up: 6789,
            },
            rtt_current_p50_nanos: DownUpOrder {
                down: Some(1000),
                up: Some(2000),
            },
            qoo: DownUpOrder {
                down: Some(95.0),
                up: Some(93.0),
            },
            tcp_retransmit_sample: DownUpOrder {
                down: TcpRetransmitSample::new(3, 300),
                up: TcpRetransmitSample::new(1, 100),
            },
            last_seen_nanos: 42,
        });

        assert_eq!(rollup.circuit_id, "10001-10004");
        assert_eq!(rollup.circuit_name, "Crew Device Group");
        assert_eq!(rollup.parent_node, "Ship-10001-Crew");
        assert_eq!(rollup.device_names, vec!["Crew Devices"]);
        assert_eq!(rollup.ip_addrs.len(), 2);
        assert_eq!(rollup.plan_mbps.down, 10.0);
        assert_eq!(rollup.bytes_per_second.down, 12345);
        assert_eq!(rollup.rtt_current_p50_nanos.up, Some(2000));
        assert_eq!(rollup.qoo.down, Some(95.0));
        assert_eq!(rollup.tcp_retransmits.down, 3);
        assert_eq!(rollup.tcp_packets.up, 100);
        assert_eq!(rollup.last_seen_nanos, 42);
    }
}
