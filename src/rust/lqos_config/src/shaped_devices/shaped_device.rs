use csv::StringRecord;
use log::error;
use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, Ipv6Addr};

use super::ShapedDevicesError;

/// Represents a row in the `ShapedDevices.csv` file.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShapedDevice {
  // Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment
  /// The ID of the circuit to which the device belongs. Circuits are 1:many,
  /// multiple devices may be in a single circuit.
  pub circuit_id: String,

  /// The name of the circuit. Since we're in a flat file, circuit names
  /// must match.
  pub circuit_name: String,

  /// The device identification, typically drawn from a management tool.
  pub device_id: String,

  /// The display name of the device.
  pub device_name: String,

  /// The parent node of the device, derived from `network.json`
  pub parent_node: String,

  /// The device's MAC address. This isn't actually used, it exists for
  /// convenient mapping/seraching.
  pub mac: String,

  /// A list of all IPv4 addresses and CIDR subnets associated with the
  /// device. For example, ("192.168.1.0", 24) is equivalent to
  /// "192.168.1.0/24"
  pub ipv4: Vec<(Ipv4Addr, u32)>,

  /// A list of all IPv4 addresses and CIDR subnets associated with the
  /// device.
  pub ipv6: Vec<(Ipv6Addr, u32)>,

  /// Minimum download: this is the bandwidth level the shaper will try
  /// to ensure is always available.
  pub download_min_mbps: u32,

  /// Minimum upload: this is the bandwidth level the shaper will try to
  /// ensure is always available.
  pub upload_min_mbps: u32,

  /// Maximum download speed, when possible.
  pub download_max_mbps: u32,

  /// Maximum upload speed when possible.
  pub upload_max_mbps: u32,

  /// Generic comments field, does nothing.
  pub comment: String,
}

impl Default for ShapedDevice {
  fn default() -> Self {
    Self {
      circuit_id: String::new(),
      circuit_name: String::new(),
      device_id: String::new(),
      device_name: String::new(),
      parent_node: String::new(),
      mac: String::new(),
      ipv4: Vec::new(),
      ipv6: Vec::new(),
      download_min_mbps: 0,
      download_max_mbps: 0,
      upload_min_mbps: 0,
      upload_max_mbps: 0,
      comment: String::new(),
    }
  }
}

impl ShapedDevice {
  pub(crate) fn from_csv(
    record: &StringRecord,
  ) -> Result<Self, ShapedDevicesError> {
    Ok(Self {
      circuit_id: record[0].to_string(),
      circuit_name: record[1].to_string(),
      device_id: record[2].to_string(),
      device_name: record[3].to_string(),
      parent_node: record[4].to_string(),
      mac: record[5].to_string(),
      ipv4: ShapedDevice::parse_ipv4(&record[6]),
      ipv6: ShapedDevice::parse_ipv6(&record[7]),
      download_min_mbps: record[8].parse().map_err(|_| {
        ShapedDevicesError::CsvEntryParseError(record[8].to_string())
      })?,
      upload_min_mbps: record[9].parse().map_err(|_| {
        ShapedDevicesError::CsvEntryParseError(record[9].to_string())
      })?,
      download_max_mbps: record[10].parse().map_err(|_| {
        ShapedDevicesError::CsvEntryParseError(record[10].to_string())
      })?,
      upload_max_mbps: record[11].parse().map_err(|_| {
        ShapedDevicesError::CsvEntryParseError(record[11].to_string())
      })?,
      comment: record[12].to_string(),
    })
  }

  pub(crate) fn parse_cidr_v4(
    address: &str,
  ) -> Result<(Ipv4Addr, u32), ShapedDevicesError> {
    if address.contains("/") {
      let split: Vec<&str> = address.split("/").collect();
      if split.len() != 2 {
        error!("Unable to parse IPv4 {address}");
        return Err(ShapedDevicesError::IPv4ParseError(address.to_string()));
      }
      return Ok((
        split[0].parse().map_err(|_| {
          ShapedDevicesError::IPv4ParseError(address.to_string())
        })?,
        split[1].parse().map_err(|_| {
          ShapedDevicesError::IPv4ParseError(address.to_string())
        })?,
      ));
    } else {
      return Ok((
        address.parse().map_err(|_| {
          ShapedDevicesError::IPv4ParseError(address.to_string())
        })?,
        32,
      ));
    }
  }

  pub(crate) fn parse_ipv4(str: &str) -> Vec<(Ipv4Addr, u32)> {
    let mut result = Vec::new();
    if str.contains(",") {
      for ip in str.split(",") {
        let ip = ip.trim();
        if let Ok((ipv4, subnet)) = ShapedDevice::parse_cidr_v4(ip) {
          result.push((ipv4, subnet));
        }
      }
    } else {
      // No Commas
      if let Ok((ipv4, subnet)) = ShapedDevice::parse_cidr_v4(str) {
        result.push((ipv4, subnet));
      }
    }

    result
  }

  pub(crate) fn parse_cidr_v6(
    address: &str,
  ) -> Result<(Ipv6Addr, u32), ShapedDevicesError> {
    if address.contains("/") {
      let split: Vec<&str> = address.split("/").collect();
      if split.len() != 2 {
        error!("Unable to parse IPv6: {address}");
        return Err(ShapedDevicesError::IPv6ParseError(address.to_string()));
      }
      return Ok((
        split[0].parse().map_err(|_| {
          ShapedDevicesError::IPv6ParseError(address.to_string())
        })?,
        split[1].parse().map_err(|_| {
          ShapedDevicesError::IPv6ParseError(address.to_string())
        })?,
      ));
    } else {
      return Ok((
        address.parse().map_err(|_| {
          ShapedDevicesError::IPv6ParseError(address.to_string())
        })?,
        128,
      ));
    }
  }

  pub(crate) fn parse_ipv6(str: &str) -> Vec<(Ipv6Addr, u32)> {
    let mut result = Vec::new();
    if str.contains(",") {
      for ip in str.split(",") {
        let ip = ip.trim();
        if let Ok((ipv6, subnet)) = ShapedDevice::parse_cidr_v6(ip) {
          result.push((ipv6, subnet));
        }
      }
    } else {
      // No Commas
      if let Ok((ipv6, subnet)) = ShapedDevice::parse_cidr_v6(str) {
        result.push((ipv6, subnet));
      }
    }

    result
  }

  pub(crate) fn to_ipv6_list(&self) -> Vec<(Ipv6Addr, u32)> {
    let mut result = Vec::new();

    for (ipv4, cidr) in &self.ipv4 {
      result.push((ipv4.to_ipv6_mapped(), cidr + 96));
    }
    result.extend_from_slice(&self.ipv6);

    result
  }
}
