use super::{QueueStructureError, tc_handle::TcHandle};
use log::error;
use lqos_utils::hex_string::read_hex_string;
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
  pub children: Vec<QueueNode>,
}

/// Provides a convenient wrapper that attempts to decode a u64 from a JSON
/// value, and returns an error if decoding fails.
macro_rules! grab_u64 {
    ($target: expr, $key: expr, $value: expr) => {
        let tmp = $value.as_u64().ok_or(QueueStructureError::U64Parse(format!("{} => {:?}", $key, $value)));
        match tmp {
            Err(e) => {
                error!("Error decoding JSON. Key: {}, Value: {:?} is not readily convertible to a u64.", $key, $value);
                return Err(e);
            }
            Ok(data) => $target = data,
        }
    };
}

/// Provides a macro to safely unwrap TC Handles and issue an error if they didn't parse
/// correctly.
macro_rules! grab_tc_handle {
  ($target: expr, $key: expr, $value: expr) => {
    let s = $value.as_str();
    if s.is_none() {
      error!("Unable to parse {:?} as a string from JSON", s);
      return Err(QueueStructureError::StringParse(format!("{:?}", $value)));
    }
    let s = s.unwrap();
    let tmp = TcHandle::from_string(s);
    if tmp.is_err() {
      error!("Unable to parse {:?} as a TC Handle", s);
      return Err(QueueStructureError::TcHandle(format!("{:?}", tmp)));
    }
    $target = tmp.unwrap();
  };
}

/// Macro to convert hex strings (e.g. 0xff) to a u32
macro_rules! grab_hex {
  ($target: expr, $key: expr, $value: expr) => {
    let s = $value.as_str();
    if s.is_none() {
      error!("Unable to parse {:?} as a string from JSON", $value);
      return Err(QueueStructureError::StringParse(format!("{:?}", s)));
    }
    let s = s.unwrap();
    let tmp = read_hex_string(s);
    if tmp.is_err() {
      error!("Unable to parse {:?} as a hex string", $value);
      return Err(QueueStructureError::HexParse(format!("{:?}", tmp)));
    }
    $target = tmp.unwrap();
  };
}

/// Macro to extract an option<string>
macro_rules! grab_string_option {
  ($target: expr, $key: expr, $value: expr) => {
    let s = $value.as_str();
    if s.is_none() {
      error!("Unable to parse {:?} as a string from JSON", $value);
      return Err(QueueStructureError::StringParse(format!("{:?}", s)));
    }
    $target = Some(s.unwrap().to_string());
  };
}

/// Macro to extract a string
macro_rules! grab_string {
  ($target: expr, $key: expr, $value: expr) => {
    let s = $value.as_str();
    if s.is_none() {
      error!("Unable to parse {:?} as a string from JSON", $value);
      return Err(QueueStructureError::StringParse(format!("{:?}", s)));
    }
    $target = s.unwrap().to_string();
  };
}

impl QueueNode {
  pub(crate) fn from_json(
    key: &str,
    value: &Value,
  ) -> Result<Self, QueueStructureError> {
    let mut result = Self::default();
    if let Value::Object(map) = value {
      for (key, value) in map.iter() {
        match key.as_str() {
          "downloadBandwidthMbps" | "maxDownload" => {
            grab_u64!(result.download_bandwidth_mbps, key.as_str(), value);
          }
          "uploadBandwidthMbps" | "maxUpload" => {
            grab_u64!(result.upload_bandwidth_mbps, key.as_str(), value);
          }
          "downloadBandwidthMbpsMin" | "minDownload" => {
            grab_u64!(result.download_bandwidth_mbps_min, key.as_str(), value);
          }
          "uploadBandwidthMbpsMin" | "minUpload" => {
            grab_u64!(result.upload_bandwidth_mbps_min, key.as_str(), value);
          }
          "classid" => {
            grab_tc_handle!(result.class_id, key.as_str(), value);
          }
          "up_classid" => {
            grab_tc_handle!(result.up_class_id, key.as_str(), value);
          }
          "classMajor" => {
            grab_hex!(result.class_major, key.as_str(), value);
          }
          "up_classMajor" => {
            grab_hex!(result.up_class_major, key.as_str(), value);
          }
          "classMinor" => {
            grab_hex!(result.class_minor, key.as_str(), value);
          }
          "cpuNum" => {
            grab_hex!(result.cpu_num, key.as_str(), value);
          }
          "up_cpuNum" => {
            grab_hex!(result.up_cpu_num, key.as_str(), value);
          }
          "parentClassID" => {
            grab_tc_handle!(result.parent_class_id, key.as_str(), value);
          }
          "up_parentClassID" => {
            grab_tc_handle!(result.up_parent_class_id, key.as_str(), value);
          }
          "circuitId" | "circuitID" => {
            grab_string_option!(result.circuit_id, key.as_str(), value);
          }
          "circuitName" => {
            grab_string_option!(result.circuit_name, key.as_str(), value);
          }
          "parentNode" | "ParentNode" => {
            grab_string_option!(result.parent_node, key.as_str(), value);
          }
          "comment" => {
            grab_string!(result.comment, key.as_str(), value);
          }
          "deviceId" | "deviceID" => {
            grab_string_option!(result.device_id, key.as_str(), value);
          }
          "deviceName" => {
            grab_string_option!(result.device_name, key.as_str(), value);
          }
          "mac" => {
            grab_string_option!(result.mac, key.as_str(), value);
          }
          "ipv4s" => {} // Ignore
          "ipv6s" => {}
          "circuits" => {
            if let Value::Array(array) = value {
              for c in array.iter() {
                let n = QueueNode::from_json(key, c);
                if n.is_err() {
                  error!("Unable to read circuit children");
                  error!("{:?}", n);
                  return Err(QueueStructureError::Circuit);
                }
                result.circuits.push(n.unwrap());
              }
            }
          }
          "devices" => {
            if let Value::Array(array) = value {
              for c in array.iter() {
                let n = QueueNode::from_json(key, c);
                if n.is_err() {
                  error!("Unable to read device children");
                  error!("{:?}", n);
                  return Err(QueueStructureError::Device);
                }
                result.devices.push(n.unwrap());
              }
            }
          }
          "children" => {
            if let Value::Object(map) = value {
              for (key, c) in map.iter() {
                let n = QueueNode::from_json(key, c);
                if n.is_err() {
                  error!("Unable to read children. Don't worry, we all feel that way sometimes.");
                  error!("{:?}", n);
                  return Err(QueueStructureError::Children);
                }
                result.circuits.push(n.unwrap());
              }
            } else {
              log::warn!("Children was not an object");
              log::warn!("{:?}", value);
            }
          }
          "idForCircuitsWithoutParentNodes" | "type" => {
            // Ignore
          }
          _ => log::error!("I don't know how to parse key: [{key}]"),
        }
      }
    } else {
      log::warn!("Unable to parse node structure for [{key}]");
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
    for c in self.children.iter() {
      result.push(c.clone());
      let children = c.to_flat();
      result.extend_from_slice(&children);
    }
    result
  }
}
