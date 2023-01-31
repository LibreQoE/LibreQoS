use super::{queue_node::QueueNode, QueueStructureError};
use log::error;
use lqos_config::EtcLqos;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub struct QueueNetwork {
  pub(crate) cpu_node: Vec<QueueNode>,
}

impl QueueNetwork {
  pub fn path() -> Result<PathBuf, QueueStructureError> {
    let cfg = EtcLqos::load();
    if cfg.is_err() {
      error!("unable to read /etc/lqos.conf");
      return Err(QueueStructureError::LqosConf);
    }
    let cfg = cfg.unwrap();
    let base_path = Path::new(&cfg.lqos_directory);
    Ok(base_path.join("queuingStructure.json"))
  }

  fn exists() -> bool {
    if let Ok(path) = QueueNetwork::path() {
      path.exists()
    } else {
      false
    }
  }

  pub(crate) fn from_json() -> Result<Self, QueueStructureError> {
    let path = QueueNetwork::path()?;
    if !QueueNetwork::exists() {
      error!("queueStructure.json does not exist yet. Try running LibreQoS?");
      return Err(QueueStructureError::FileNotFound);
    }
    let raw_string = std::fs::read_to_string(path)
      .map_err(|_| QueueStructureError::FileNotFound)?;
    let mut result = Self { cpu_node: Vec::new() };
    let json: Value = serde_json::from_str(&raw_string)
      .map_err(|_| QueueStructureError::FileNotFound)?;
    if let Value::Object(map) = &json {
      if let Some(network) = map.get("Network") {
        if let Value::Object(map) = network {
          for (key, value) in map.iter() {
            result.cpu_node.push(QueueNode::from_json(key, value)?);
          }
        } else {
          error!("Unable to parse JSON for queueStructure");
          return Err(QueueStructureError::JsonError);
        }
      } else {
        error!("Unable to parse JSON for queueStructure");
        return Err(QueueStructureError::JsonError);
      }
    } else {
      error!("Unable to parse JSON for queueStructure");
      return Err(QueueStructureError::JsonError);
    }

    Ok(result)
  }

  pub fn to_flat(&self) -> Vec<QueueNode> {
    let mut result = Vec::new();
    for cpu in self.cpu_node.iter() {
      result.push(cpu.clone());
      let children = cpu.to_flat();
      result.extend_from_slice(&children);
    }
    for c in result.iter_mut() {
      c.circuits.clear();
      c.devices.clear();
    }
    result
  }
}
