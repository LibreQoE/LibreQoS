mod session_buffer;
mod min_max;
mod system_stats;
use crate::{transport_data::{StatsHost, StatsSummary, StatsRttSummary, StatsTreeNode, StatsSubmission, StatsTotals}, submission_queue::{new_submission, comm_channel::SenderChannelMessage}};
use self::min_max::{MinMaxAvgPair, MinMaxAvg};
pub(crate) use session_buffer::{StatsSession, SESSION_BUFFER};
use lqos_utils::unix_time::unix_now;
use tokio::sync::mpsc::Sender;
use std::{collections::HashMap, net::IpAddr};
use super::{HostSummary, NetworkTreeEntry};

pub(crate) async fn collate_stats(comm_tx: Sender<SenderChannelMessage>) {
    let timestamp = unix_now().unwrap_or(0);
    if timestamp == 0 {
        return; // We're not ready
    }

    let mut writer = SESSION_BUFFER.lock().await;
    if writer.is_empty() {
        return; // Nothing to do
    }

    // Collate total stats for the period
    let bps: Vec<(u64, u64)> = writer
        .iter()
        .map(|e| e.throughput.bits_per_second)
        .collect();
    let pps: Vec<(u64, u64)> = writer
        .iter()
        .map(|e| e.throughput.packets_per_second)
        .collect();
    let sbps: Vec<(u64, u64)> = writer
        .iter()
        .map(|e| e.throughput.shaped_bits_per_second)
        .collect();
    let bits_per_second = MinMaxAvgPair::from_slice(&bps);
    let packets_per_second = MinMaxAvgPair::from_slice(&pps);
    let shaped_bits_per_second = MinMaxAvgPair::from_slice(&sbps);

    // Iterate hosts gathering min/max data
    let mut hosts_accumulator: HashMap<IpAddr, Vec<&HostSummary>> = HashMap::new();
    let mut tree_accumulator: HashMap<String, Vec<(usize, &NetworkTreeEntry)>> = HashMap::new();
    writer.iter().for_each(|e| {
        e.throughput.hosts.iter().for_each(|host| {
            if let Some(hosts) = hosts_accumulator.get_mut(&host.ip) {
                hosts.push(host);
            } else {
                hosts_accumulator.insert(host.ip, vec![host]);
            }
        });

        e.network_tree.iter().for_each(|(index, node)| {
            if let Some(t) = tree_accumulator.get_mut(&node.name) {
                t.push((*index, node));
            } else {
                tree_accumulator.insert(node.name.clone(), vec![(*index, node)]);
            }
        });
    });

    // Get min/max data per IP
    let mut stats_hosts = Vec::new();
    for (ip, host) in hosts_accumulator.into_iter() {
        let bits = MinMaxAvgPair::from_slice(
            &host
                .iter()
                .map(|h| (h.bits_per_second.0, h.bits_per_second.1))
                .collect::<Vec<(u64, u64)>>(),
        );
        let rtt = MinMaxAvg::from_slice(
            &host
                .iter()
                .map(|h| (h.median_rtt * 100.0) as u32)
                .collect::<Vec<u32>>(),
        );

        let sh = StatsHost {
            ip_address: ip.to_string(),
            circuit_id: host[0].circuit_id.clone(),
            bits: StatsSummary{ min: (bits.down.min, bits.up.min), max: (bits.down.max, bits.up.max), avg: (bits.down.avg, bits.up.avg) },
            rtt: StatsRttSummary{ min: rtt.min, max: rtt.max, avg: rtt.avg },
        };
        stats_hosts.push(sh);
    }

    // Get network tree min/max data
    let mut tree_entries = Vec::new();
    for (name, nodes) in tree_accumulator.into_iter() {
        let bits = MinMaxAvgPair::from_slice(
            &nodes
                .iter()
                .map(|(_i, n)| (n.current_throughput.0, n.current_throughput.1))
                .collect::<Vec<(u32, u32)>>(),
        );
        let rtt = MinMaxAvg::from_slice(
            &nodes
                .iter()
                .map(|(_i, n)| (n.rtts.2) as u32)
                .collect::<Vec<u32>>(),
        );

        let n = StatsTreeNode {
            index: nodes[0].0,
            name: name.to_string(),
            max_throughput: nodes[0].1.max_throughput,
            current_throughput: StatsSummary{ min: (bits.down.min.into(), bits.up.min.into()), max: (bits.down.max.into(), bits.up.max.into()), avg: (bits.down.avg.into(), bits.up.avg.into()) },
            rtt: StatsRttSummary{ min: rtt.min, max: rtt.max, avg: rtt.avg },
            parents: nodes[0].1.parents.clone(),
            immediate_parent: nodes[0].1.immediate_parent,
            node_type: nodes[0].1.node_type.clone(),
        };
        tree_entries.push(n);
    }

    // Add to the submissions queue
    let (cpu, ram) = system_stats::get_cpu_ram().await;
    new_submission(StatsSubmission {
        timestamp,
        totals: Some(StatsTotals {
            bits: StatsSummary {
                min: (bits_per_second.down.min, bits_per_second.up.min),
                max: (bits_per_second.down.max, bits_per_second.up.max),
                avg: (bits_per_second.down.avg, bits_per_second.up.avg),
            },
            shaped_bits: StatsSummary {
                min: (shaped_bits_per_second.down.min, shaped_bits_per_second.up.min),
                max: (shaped_bits_per_second.down.max, shaped_bits_per_second.up.max),
                avg: (shaped_bits_per_second.down.avg, shaped_bits_per_second.up.avg),
            },
            packets: StatsSummary {
                min: (packets_per_second.down.min, packets_per_second.up.min),
                max: (packets_per_second.down.max, packets_per_second.up.max),
                avg: (packets_per_second.down.avg, packets_per_second.up.avg),
            },
        }),
        cpu_usage: Some(cpu),
        ram_percent: Some(ram),
        hosts: Some(stats_hosts),
        tree: Some(tree_entries),
        uisp_devices: None,
    }, comm_tx).await;

    // Clear the collection buffer
    writer.clear();
}
