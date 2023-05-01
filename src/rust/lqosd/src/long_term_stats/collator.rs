use lqos_utils::unix_time::unix_now;
use once_cell::sync::Lazy;
use sysinfo::{System, SystemExt};

use super::{
    collation_utils::{MinMaxAvg, MinMaxAvgPair},
    submission::new_submission,
    tree::{get_network_tree, NetworkTreeEntry},
};
use crate::long_term_stats::data_collector::SESSION_BUFFER;
use std::{collections::HashMap, net::IpAddr, sync::Mutex};

#[derive(Debug, Clone)]
pub(crate) struct StatsSubmission {
    pub(crate) timestamp: u64,
    pub(crate) bits_per_second: MinMaxAvgPair<u64>,
    pub(crate) shaped_bits_per_second: MinMaxAvgPair<u64>,
    pub(crate) packets_per_second: MinMaxAvgPair<u64>,
    pub(crate) hosts: Vec<SubmissionHost>,
    pub(crate) tree: Vec<NetworkTreeEntry>,
}

#[derive(Debug, Clone)]
pub(crate) struct SubmissionHost {
    pub(crate) circuit_id: String,
    pub(crate) ip_address: IpAddr,
    pub(crate) bits_per_second: MinMaxAvgPair<u64>,
    pub(crate) median_rtt: MinMaxAvg<u32>,
    pub(crate) tree_parent_indices: Vec<usize>,
    pub(crate) device_id: String,
    pub(crate) parent_node: String,
    pub(crate) device_name: String,
    pub(crate) circuit_name: String,
    pub(crate) mac: String,
}

static SYS: Lazy<Mutex<System>> = Lazy::new(|| Mutex::new(System::new_all()));

fn get_cpu_ram() -> (Vec<u32>, u32) {
    use sysinfo::CpuExt;
    let mut lock = SYS.lock().unwrap();
    lock.refresh_cpu();
    lock.refresh_memory();

    let cpus: Vec<u32> = lock
        .cpus()
        .iter()
        .map(|cpu| cpu.cpu_usage() as u32) // Always rounds down
        .collect();

    let memory = (lock.used_memory() as f32 / lock.total_memory() as f32) * 100.0;

    //println!("cpu: {:?}, ram: {}", cpus, memory);

    (cpus, memory as u32)
}

impl From<StatsSubmission> for lts_client::transport_data::StatsSubmission {
    fn from(value: StatsSubmission) -> Self {
        let (cpu, ram) = get_cpu_ram();
        Self {
            cpu_usage: cpu,
            ram_percent: ram,
            timestamp: value.timestamp,
            totals: Some(value.clone().into()),
            hosts: Some(value.hosts.into_iter().map(Into::into).collect()),
            tree: Some(value.tree.into_iter().map(Into::into).collect()),
        }
    }
}

impl From<NetworkTreeEntry> for lts_client::transport_data::StatsTreeNode {
    fn from(value: NetworkTreeEntry) -> Self {
        Self {
            name: value.name.clone(),
            max_throughput: value.max_throughput,
            current_throughput: value.current_throughput,
            parents: value.parents,
            immediate_parent: value.immediate_parent,
            node_type: value.node_type,
            rtt: value.rtts,
        }
    }
}

impl From<StatsSubmission> for lts_client::transport_data::StatsTotals {
    fn from(value: StatsSubmission) -> Self {
        Self {
            bits: value.bits_per_second.into(),
            shaped_bits: value.shaped_bits_per_second.into(),
            packets: value.packets_per_second.into(),
        }
    }
}

impl From<MinMaxAvgPair<u64>> for lts_client::transport_data::StatsSummary {
    fn from(value: MinMaxAvgPair<u64>) -> Self {
        Self {
            min: (value.down.min, value.up.min),
            max: (value.down.max, value.up.max),
            avg: (value.down.avg, value.up.avg),
        }
    }
}

impl From<MinMaxAvg<u32>> for lts_client::transport_data::StatsRttSummary {
    fn from(value: MinMaxAvg<u32>) -> Self {
        Self {
            min: value.min,
            max: value.max,
            avg: value.avg,
        }
    }
}

impl From<SubmissionHost> for lts_client::transport_data::StatsHost {
    fn from(value: SubmissionHost) -> Self {
        Self {
            circuit_id: value.circuit_id.to_string(),
            ip_address: value.ip_address.to_string(),
            bits: value.bits_per_second.into(),
            rtt: value.median_rtt.into(),
            tree_indices: value.tree_parent_indices,
            device_id: value.device_id,
            parent_node: value.parent_node,
            circuit_name: value.circuit_name,
            device_name: value.device_name,
            mac: value.mac,
        }
    }
}

/// Every (n) seconds, collate the accumulated stats buffer
/// into a current statistics block (min/max/avg format)
/// ready for submission to the stats system.
///
/// (n) is defined in /etc/lqos.conf in the `collation_period_seconds`
/// field of the `[long_term_stats]` section.
pub(crate) async fn collate_stats() {
    // Obtain exclusive access to the session
    let mut writer = SESSION_BUFFER.lock().await;
    if writer.is_empty() {
        // Nothing to do - so exit
        return;
    }

    // Collate total stats for the period
    let bps: Vec<(u64, u64)> = writer.iter().map(|e| e.bits_per_second).collect();
    let pps: Vec<(u64, u64)> = writer.iter().map(|e| e.packets_per_second).collect();
    let sbps: Vec<(u64, u64)> = writer.iter().map(|e| e.shaped_bits_per_second).collect();
    let bits_per_second = MinMaxAvgPair::from_slice(&bps);
    let packets_per_second = MinMaxAvgPair::from_slice(&pps);
    let shaped_bits_per_second = MinMaxAvgPair::from_slice(&sbps);

    let mut submission = StatsSubmission {
        timestamp: unix_now().unwrap_or(0),
        bits_per_second,
        shaped_bits_per_second,
        packets_per_second,
        hosts: Vec::new(),
        tree: get_network_tree(),
    };

    // Collate host stats
    let mut host_accumulator =
        HashMap::<(&IpAddr, &String, &String, &String, &String, &String, &String), Vec<(u64, u64, f32, Vec<usize>)>>::new();
    writer.iter().for_each(|session| {
        session.hosts.iter().for_each(|host| {
            if let Some(ha) = host_accumulator.get_mut(&(
                &host.ip_address, &host.circuit_id, &host.device_id, &host.parent_node, &host.circuit_name, &host.device_name, &host.mac)
            ) {
                ha.push((
                    host.bits_per_second.0,
                    host.bits_per_second.1,
                    host.median_rtt,
                    host.tree_parent_indices.clone(),
                ));
            } else {
                host_accumulator.insert(
                    (&host.ip_address, &host.circuit_id, &host.device_id, &host.parent_node, &host.circuit_name, &host.device_name, &host.mac),
                    vec![(
                        host.bits_per_second.0,
                        host.bits_per_second.1,
                        host.median_rtt,
                        host.tree_parent_indices.clone(),
                    )],
                );
            }
        });
    });

    for ((ip, circuit, device_id, parent_node, circuit_name, device_name, mac), data) in host_accumulator.iter() {
        let bps: Vec<(u64, u64)> = data.iter().map(|(d, u, _rtt, _tree)| (*d, *u)).collect();
        let bps = MinMaxAvgPair::<u64>::from_slice(&bps);
        let fps: Vec<u32> = data
            .iter()
            .map(|(_d, _u, rtt, _tree)| (*rtt * 100.0) as u32)
            .collect();
        let fps = MinMaxAvg::<u32>::from_slice(&fps);
        let tree = data
            .iter()
            .cloned()
            .map(|(_d, _u, _rtt, tree)| tree)
            .next()
            .unwrap_or(Vec::new());
        
        submission.hosts.push(SubmissionHost {
            circuit_id: circuit.to_string(),
            ip_address: **ip,
            bits_per_second: bps,
            median_rtt: fps,
            tree_parent_indices: tree,
            device_id: device_id.to_string(),
            parent_node: parent_node.to_string(),
            circuit_name: circuit_name.to_string(),
            device_name: device_name.to_string(),
            mac: mac.to_string(),
        });
    }

    // Remove all gathered stats
    writer.clear();

    // Drop the lock
    std::mem::drop(writer);

    // Submit
    new_submission(submission).await;
}
