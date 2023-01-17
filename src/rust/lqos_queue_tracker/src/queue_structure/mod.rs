mod queing_structure_json_monitor;
mod queue_network;
mod queue_node;
use anyhow::Result;
pub use queing_structure_json_monitor::spawn_queue_structure_monitor;
pub(crate) use queing_structure_json_monitor::QUEUE_STRUCTURE;
use queue_network::QueueNetwork;
use queue_node::QueueNode;

fn read_hex_string(s: &str) -> Result<u32> {
    Ok(u32::from_str_radix(&s.replace("0x", ""), 16)?)
}

pub(crate) fn read_queueing_structure() -> Result<Vec<QueueNode>> {
    let network = QueueNetwork::from_json()?;
    Ok(network.to_flat())
}
