mod queing_structure_json_monitor;
mod queue_network;
mod queue_node;
use log::error;
pub use queing_structure_json_monitor::spawn_queue_structure_monitor;
pub(crate) use queing_structure_json_monitor::QUEUE_STRUCTURE;
use queue_network::QueueNetwork;
use queue_node::QueueNode;
use thiserror::Error;

fn read_hex_string(s: &str) -> Result<u32, HexParseError> {
  let result = u32::from_str_radix(&s.replace("0x", ""), 16);
  match result {
    Ok(data) => Ok(data),
    Err(e) => {
      error!("Unable to convert {s} to a u32");
      error!("{:?}", e);
      Err(HexParseError::ParseError)
    }
  }
}

pub(crate) fn read_queueing_structure(
) -> Result<Vec<QueueNode>, QueueStructureError> {
  // Note: the ? is allowed because the sub-types return a QueueStructureError and handle logging.
  let network = QueueNetwork::from_json()?;
  let flattened = network.to_flat();
  Ok(flattened)
}

#[derive(Error, Debug)]
pub enum HexParseError {
  #[error("Unable to decode string into valid hex")]
  ParseError,
}

#[derive(Error, Debug)]
pub enum QueueStructureError {
  #[error("Unable to parse node structure from JSON")]
  JsonKeyUnparseable(String),
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
