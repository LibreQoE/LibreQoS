use crate::lts2_sys::lts2_client::ingestor::commands::IngestorCommand;
use crate::lts2_sys::shared_types::{FlowCount, IngestSession, NetworkTree, OneWayFlow, ShapedDevices, ShaperThroughput, ShaperUtilization, TwoWayFlow};

pub(crate) fn add_general(message: &mut IngestSession, queue: &mut Vec<IngestorCommand>) {
    while let Some(msg) = queue.pop() {
        match msg {
            IngestorCommand::TotalThroughput {
                timestamp,
                download_bytes,
                upload_bytes,
                shaped_download_bytes,
                shaped_upload_bytes,
                packets_down,
                packets_up,
                tcp_packets_down,
                tcp_packets_up,
                udp_packets_down,
                udp_packets_up,
                icmp_packets_down,
                icmp_packets_up,
                min_rtt,
                max_rtt,
                median_rtt,
                tcp_retransmits_down,
                tcp_retransmits_up,
                cake_marks_down,
                cake_marks_up,
                cake_drops_down,
                cake_drops_up,
            } => {
                message.shaper_throughput.push(ShaperThroughput {
                    tick: timestamp,
                    bytes_per_second_down: download_bytes as i64,
                    bytes_per_second_up: upload_bytes as i64,
                    shaped_bytes_per_second_down: shaped_download_bytes as i64,
                    shaped_bytes_per_second_up: shaped_upload_bytes as i64,
                    packets_down: packets_down as i64,
                    packets_up: packets_up as i64,
                    tcp_packets_down: tcp_packets_down as i64,
                    tcp_packets_up: tcp_packets_up as i64,
                    udp_packets_down: udp_packets_down as i64,
                    udp_packets_up: udp_packets_up as i64,
                    icmp_packets_down: icmp_packets_down as i64,
                    icmp_packets_up: icmp_packets_up as i64,
                    max_rtt,
                    min_rtt,
                    median_rtt,
                    tcp_retransmits_down,
                    tcp_retransmits_up,
                    cake_marks_down,
                    cake_marks_up,
                    cake_drops_down,
                    cake_drops_up,
                });
            }
            IngestorCommand::ShapedDevices { timestamp, devices } => {
                message.shaped_devices.push(ShapedDevices {
                    tick: timestamp,
                    blob: devices,
                });
            }
            IngestorCommand::NetworkTree { timestamp, tree } => {
                message.network_tree.push(NetworkTree {
                    tick: timestamp,
                    blob: tree,
                });
            }
            IngestorCommand::ShaperUtilization { tick, average_cpu, peak_cpu, memory_percent } => {
                if message.shaper_utilization.is_none() {
                    message.shaper_utilization = Some(Vec::new());
                }
                if let Some(msg) = &mut message.shaper_utilization {
                    msg.push(ShaperUtilization {
                        tick,
                        average_cpu,
                        peak_cpu,
                        memory_percent,
                    });
                }
            }
            IngestorCommand::OneWayFlow { start_time, end_time, local_ip, remote_ip, dst_port, src_port, bytes, protocol, circuit_hash } => {
                if message.one_way_flows.is_none() {
                    message.one_way_flows = Some(Vec::new());
                }
                if let Some(msg) = &mut message.one_way_flows {
                    msg.push(OneWayFlow {
                        start_time,
                        end_time,
                        local_ip,
                        remote_ip,
                        dst_port,
                        src_port,
                        bytes,
                        protocol,
                        circuit_hash,
                    });
                }
            }
            IngestorCommand::TwoWayFlow { start_time, end_time, local_ip, remote_ip, dst_port, src_port, bytes_down, bytes_up, retransmit_times_down, retransmit_times_up, protocol, rtt1, rtt2, circuit_hash } => {
                if message.two_way_flows.is_none() {
                    message.two_way_flows = Some(Vec::new());
                }
                if let Some(msg) = &mut message.two_way_flows {
                    msg.push(TwoWayFlow {
                        start_time,
                        end_time,
                        local_ip,
                        remote_ip,
                        dst_port,
                        src_port,
                        bytes_down,
                        bytes_up,
                        retransmit_times_down,
                        retransmit_times_up,
                        protocol,
                        rtt: [ rtt1, rtt2 ],
                        circuit_hash,
                    });
                }
            }
            IngestorCommand::AllowSubnet(subnet) => {
                if message.allowed_ips.is_none() {
                    message.allowed_ips = Some(Vec::new());
                }
                if let Some(msg) = &mut message.allowed_ips {
                    msg.push(subnet);
                }
            }
            IngestorCommand::IgnoreSubnet(subnet) => {
                if message.ignored_ips.is_none() {
                    message.ignored_ips = Some(Vec::new());
                }
                if let Some(msg) = &mut message.ignored_ips {
                    msg.push(subnet);
                }
            }
            IngestorCommand::BlackboardJson(json) => {
                message.blackboard_json = Some(json);
            }
            IngestorCommand::FlowCount{ timestamp, flow_count} => {
                if message.flow_count.is_none() {
                    message.flow_count = Some(Vec::new());
                }
                if let Some(msg) = &mut message.flow_count {
                    msg.push(FlowCount {
                        timestamp,
                        count: flow_count,
                    });
                }
            }
            _ => {}
        }
    }
}
