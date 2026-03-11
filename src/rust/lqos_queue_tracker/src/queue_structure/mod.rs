mod queing_structure_json_monitor;
mod queue_network;
mod queue_node;
pub use queing_structure_json_monitor::spawn_queue_structure_monitor;
pub use queing_structure_json_monitor::{QUEUE_STRUCTURE, QUEUE_STRUCTURE_CHANGED_STORMGUARD};
use queue_network::QueueNetwork;
pub(crate) use queue_node::QueueNode;
use thiserror::Error;

pub(crate) fn read_queueing_structure() -> Result<Vec<QueueNode>, QueueStructureError> {
    // Note: the ? is allowed because the sub-types return a QueueStructureError and handle logging.
    let network = QueueNetwork::from_json()?;
    let flattened = network.to_flat();
    Ok(flattened)
}

#[derive(Error, Debug)]
pub enum QueueStructureError {
    #[error("unable to parse u64")]
    U64Parse(String),
    #[error("Unable to retrieve string from JSON")]
    StringParse(String),
    #[error("Unable to convert string to TC Handle")]
    TcHandle(String),
    #[error("Unable to convert string to u32 via hex")]
    HexParse(String),
    #[error("Error reading child circuit")]
    Circuit,
    #[error("Error reading child device")]
    Device,
    #[error("Error reading child's children")]
    Children,
    #[error("Unable to read configuration from /etc/lqos.conf")]
    LqosConf,
    #[error("Unable to access queueingStructure.json")]
    FileNotFound,
    #[error("Unable to read JSON")]
    JsonError,
}
