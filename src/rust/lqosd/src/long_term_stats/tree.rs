use lqos_bus::long_term_stats::StatsTreeNode;
use lqos_config::NetworkJsonNode;

use crate::shaped_devices_tracker::NETWORK_JSON;

#[derive(Debug, Clone)]
pub(crate) struct NetworkTreeEntry {
    pub(crate) name: String,
    pub(crate) max_throughput: (u32, u32),
    pub(crate) current_throughput: (u32, u32),
    pub(crate) rtts: (u16, u16, u16),
    pub(crate) parents: Vec<usize>,
    pub(crate) immediate_parent: Option<usize>,
    pub(crate) node_type: Option<String>,
}

impl From<&NetworkJsonNode> for NetworkTreeEntry {
    fn from(value: &NetworkJsonNode) -> Self {
        let mut max = 0;
        let mut min = if value.rtts.is_empty() {
            0
        } else {
            u16::MAX
        };
        let mut sum = 0;
        for n in value.rtts.iter() {
            let n = *n;
            sum += n;
            if n < min { min = n; }
            if n > max { max = n; }
        }
        let avg = sum.checked_div(value.rtts.len() as u16).unwrap_or(0);

        Self {
            name: value.name.clone(),
            max_throughput: value.max_throughput,
            parents: value.parents.clone(),
            immediate_parent: value.immediate_parent,
            current_throughput: (
                value.current_throughput.0.load(std::sync::atomic::Ordering::Relaxed) as u32,
                value.current_throughput.1.load(std::sync::atomic::Ordering::Relaxed) as u32,
            ),
            node_type: value.node_type.clone(),
            rtts: (min, max, avg),
        }
    }
}

impl From<&NetworkTreeEntry> for StatsTreeNode {
    fn from(value: &NetworkTreeEntry) -> Self {
        Self {
            name: value.name.clone(),
            max_throughput: value.max_throughput,
            current_throughput: value.current_throughput,
            parents: value.parents.clone(),
            immediate_parent: value.immediate_parent,
            node_type: value.node_type.clone(),
            rtt: value.rtts,
        }
    }
}

pub(crate) fn get_network_tree() -> Vec<NetworkTreeEntry> {
    if let Ok(reader) = NETWORK_JSON.read() {
        return reader
            .nodes
            .iter()
            .map(|n| n.into())
            .collect::<Vec<NetworkTreeEntry>>();
    }
    Vec::new()
}