use super::read_hex_string;
use anyhow::{Error, Result};
use lqos_bus::TcHandle;
use serde_json::Value;

#[derive(Default, Clone, Debug)]
pub struct QueueNode {
    pub download_bandwidth_mbps: u64,
    pub upload_bandwidth_mbps: u64,
    pub download_bandwidth_mbps_min: u64,
    pub upload_bandwidth_mbps_min: u64,
    pub class_id: TcHandle,
    pub up_class_id: TcHandle,
    pub parent_class_id: TcHandle,
    pub up_parent_class_id: TcHandle,
    pub class_major: u32,
    pub up_class_major: u32,
    pub class_minor: u32,
    pub cpu_num: u32,
    pub up_cpu_num: u32,
    pub circuits: Vec<QueueNode>,
    pub circuit_id: Option<String>,
    pub circuit_name: Option<String>,
    pub parent_node: Option<String>,
    pub devices: Vec<QueueNode>,
    pub comment: String,
    pub device_id: Option<String>,
    pub device_name: Option<String>,
    pub mac: Option<String>,
}

impl QueueNode {
    pub(crate) fn from_json(key: &str, value: &Value) -> Result<Self> {
        let mut result = Self::default();
        if let Value::Object(map) = value {
            for (key, value) in map.iter() {
                match key.as_str() {
                    "downloadBandwidthMbps" | "maxDownload" => {
                        result.download_bandwidth_mbps = value.as_u64().unwrap()
                    }
                    "uploadBandwidthMbps" | "maxUpload" => {
                        result.upload_bandwidth_mbps = value.as_u64().unwrap()
                    }
                    "downloadBandwidthMbpsMin" | "minDownload" => {
                        result.download_bandwidth_mbps_min = value.as_u64().unwrap()
                    }
                    "uploadBandwidthMbpsMin" | "minUpload" => {
                        result.upload_bandwidth_mbps_min = value.as_u64().unwrap()
                    }
                    "classid" => {
                        result.class_id =
                            TcHandle::from_string(&value.as_str().unwrap().to_string())?
                    }
                    "up_classid" => {
                        result.up_class_id =
                            TcHandle::from_string(value.as_str().unwrap().to_string())?
                    }
                    "classMajor" => result.class_major = read_hex_string(value.as_str().unwrap())?,
                    "up_classMajor" => {
                        result.up_class_major = read_hex_string(value.as_str().unwrap())?
                    }
                    "classMinor" => result.class_minor = read_hex_string(value.as_str().unwrap())?,
                    "cpuNum" => result.cpu_num = read_hex_string(value.as_str().unwrap())?,
                    "up_cpuNum" => result.up_cpu_num = read_hex_string(value.as_str().unwrap())?,
                    "parentClassID" => {
                        result.parent_class_id =
                            TcHandle::from_string(value.as_str().unwrap().to_string())?
                    }
                    "up_parentClassID" => {
                        result.up_parent_class_id =
                            TcHandle::from_string(value.as_str().unwrap().to_string())?
                    }
                    "circuitId" | "circuitID" => {
                        result.circuit_id = Some(value.as_str().unwrap().to_string())
                    }
                    "circuitName" => {
                        result.circuit_name = Some(value.as_str().unwrap().to_string())
                    }
                    "parentNode" | "ParentNode" => {
                        result.parent_node = Some(value.as_str().unwrap().to_string())
                    }
                    "comment" => result.comment = value.as_str().unwrap().to_string(),
                    "deviceId" | "deviceID" => {
                        result.device_id = Some(value.as_str().unwrap().to_string())
                    }
                    "deviceName" => result.device_name = Some(value.as_str().unwrap().to_string()),
                    "mac" => result.mac = Some(value.as_str().unwrap().to_string()),
                    "ipv4s" => {} // Ignore
                    "ipv6s" => {}
                    "circuits" => {
                        if let Value::Array(array) = value {
                            for c in array.iter() {
                                result.circuits.push(QueueNode::from_json(key, c)?);
                            }
                        }
                    }
                    "devices" => {
                        if let Value::Array(array) = value {
                            for c in array.iter() {
                                result.devices.push(QueueNode::from_json(key, c)?);
                            }
                        }
                    }
                    "children" => {} // Ignore for now
                    _ => log::error!("I don't know how to parse key: [{key}]"),
                }
            }
        } else {
            return Err(Error::msg(format!(
                "Unable to parse node structure for [{key}]"
            )));
        }
        Ok(result)
    }

    pub(crate) fn to_flat(&self) -> Vec<QueueNode> {
        let mut result = Vec::new();
        for c in self.circuits.iter() {
            result.push(c.clone());
            let children = c.to_flat();
            result.extend_from_slice(&children);
        }
        for c in self.devices.iter() {
            result.push(c.clone());
            let children = c.to_flat();
            result.extend_from_slice(&children);
        }
        result
    }
}

#[cfg(test)]
mod test {
    use super::*;

    const EXAMPLE_QUEUE_STRUCTURE_WITH_CHILDREN: &str = include_str!("./example_queue_with_children.test.json");
    const EXAMPLE_QUEUE_STRUCTURE_NO_CHILDREN: &str = include_str!("./example_queue_flat.test.json");

    fn try_load_queue_structure(raw_string: &str) {
        let mut result = super::super::QueueNetwork {
            cpu_node: Vec::new(),
        };
        let json: Value = serde_json::from_str(&raw_string).unwrap();
        if let Value::Object(map) = &json {
            if let Some(network) = map.get("Network") {
                if let Value::Object(map) = network {
                    for (key, value) in map.iter() {
                        result.cpu_node.push(QueueNode::from_json(&key, value).unwrap());
                    }
                } else {
                    panic!("Unable to parse network object structure");
                }
            } else {
                panic!("Network entry not found");
            }
        } else {
            panic!("Unable to parse queueStructure.json");
        }
    }

    #[test]
    fn load_queue_structure_no_children() {
        try_load_queue_structure(EXAMPLE_QUEUE_STRUCTURE_NO_CHILDREN);
    }

    #[test]
    fn load_queue_structure_with_children() {
        try_load_queue_structure(EXAMPLE_QUEUE_STRUCTURE_WITH_CHILDREN);
    }
}