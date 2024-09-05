use crate::{deserialize_tc_tree, queue_types::QueueType};
use tracing::{error, info};
use lqos_bus::TcHandle;
use std::process::Command;
use thiserror::Error;

const TC: &str = "/sbin/tc";

pub fn read_all_queues_from_interface(
  interface: &str
) -> Result<Vec<QueueType>, QueueReaderError> {
  let command_output = Command::new(TC)
      .args([
        "-s",
        "-j",
        "qdisc",
        "show",
        "dev",
        interface,
      ])
      .output()
      .map_err(|e| {
        info!("Failed to poll TC for queues: {interface}");
        info!("{:?}", e);
        QueueReaderError::CommandError
      })?;

  let raw_json = String::from_utf8(command_output.stdout)
      .map_err(|e| {
        info!("Failed to convert byte stream to UTF-8 string");
        info!("{:?}", e);
        QueueReaderError::Utf8Error
      })?;
  let result = deserialize_tc_tree(&raw_json)
      .map_err(|e| {
        info!("Failed to deserialize TC tree result.");
        info!("{:?}", e);
        QueueReaderError::Deserialization
      })?;

  Ok(result)
}

pub fn read_named_queue_from_interface(
  interface: &str,
  tc_handle: TcHandle,
) -> Result<Vec<QueueType>, QueueReaderError> {
  let command_output = Command::new(TC)
    .args([
      "-s",
      "-j",
      "qdisc",
      "show",
      "dev",
      interface,
      "parent",
      &tc_handle.to_string(),
    ])
    .output();

  if command_output.is_err() {
    error!(
      "Failed to call process tc -s -j qdisc show dev {interface} parent {}",
      &tc_handle.to_string()
    );
    error!("{:?}", command_output);
    return Err(QueueReaderError::CommandError);
  }
  let command_output = command_output.unwrap();

  let json = String::from_utf8(command_output.stdout);
  if json.is_err() {
    error!("Failed to convert byte stream to UTF-8 string");
    error!("{:?}", json);
    return Err(QueueReaderError::Utf8Error);
  }
  let json = json.unwrap();
  let result = deserialize_tc_tree(&json);
  if result.is_err() {
    error!("Failed to deserialize TC tree result.");
    error!("{:?}", result);
    return Err(QueueReaderError::Deserialization);
  }
  Ok(result.unwrap())
}

#[derive(Error, Debug)]
pub enum QueueReaderError {
  #[error("Subprocess call failed")]
  CommandError,
  #[error("Failed to convert bytes to valid UTF-8")]
  Utf8Error,
  #[error("Deserialization Error")]
  Deserialization,
}
