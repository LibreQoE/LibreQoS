use std::path::{PathBuf, Path};
use super::queue_node::QueueNode;
use anyhow::{Result, Error};
use lqos_config::EtcLqos;
use serde_json::Value;

pub struct QueueNetwork {
    cpu_node: Vec<QueueNode>,
}

impl QueueNetwork {
    pub fn path() -> Result<PathBuf> {
        let cfg = EtcLqos::load()?;
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

    pub(crate) fn from_json() -> Result<Self> {
        let path = QueueNetwork::path()?;
        if !QueueNetwork::exists() {
            return Err(Error::msg(
                "queueStructure.json does not exist yet. Try running LibreQoS?",
            ));
        }
        let raw_string = std::fs::read_to_string(path)?;
        let mut result = Self {
            cpu_node: Vec::new(),
        };
        let json: Value = serde_json::from_str(&raw_string)?;
        if let Value::Object(map) = &json {
            if let Some(network) = map.get("Network") {
                if let Value::Object(map) = network {
                    for (key, value) in map.iter() {
                        result.cpu_node.push(QueueNode::from_json(&key, value)?);
                    }
                } else {
                    return Err(Error::msg("Unable to parse network object structure"));
                }
            } else {
                return Err(Error::msg("Network entry not found"));
            }
        } else {
            return Err(Error::msg("Unable to parse queueStructure.json"));
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
