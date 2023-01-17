mod queue_node;
mod queue_network;
mod queing_structure_json_monitor;
use anyhow::Result;
use queue_node::QueueNode;
use queue_network::QueueNetwork;
pub use queing_structure_json_monitor::spawn_queue_structure_monitor;
pub(crate) use queing_structure_json_monitor::QUEUE_STRUCTURE;

fn read_hex_string(s: &str) -> Result<u32> {
    Ok(u32::from_str_radix(&s.replace("0x", ""), 16)?)
}

pub(crate) fn read_queueing_structure() -> Result<Vec<QueueNode>> {
    let network = QueueNetwork::from_json()?;
    Ok(network.to_flat())
}