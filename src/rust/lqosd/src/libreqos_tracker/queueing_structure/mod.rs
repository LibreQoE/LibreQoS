mod reader;
use anyhow::Result;

pub use reader::{QueueNode, QueueNetwork};

pub fn read_queueing_structure() -> Result<Vec<reader::QueueNode>> {
    let network = reader::QueueNetwork::from_json()?;
    Ok(network.to_flat())
}