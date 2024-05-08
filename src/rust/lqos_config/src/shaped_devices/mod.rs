mod serializable;
mod shaped_device;
use crate::SUPPORTED_CUSTOMERS;
use csv::{QuoteStyle, ReaderBuilder, WriterBuilder};
use log::error;
use serializable::SerializableShapedDevice;
pub use shaped_device::ShapedDevice;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Provides handling of the `ShapedDevices.csv` file that maps
/// circuits to traffic shaping.
pub struct ConfigShapedDevices {
  /// List of all devices subject to traffic shaping.
  pub devices: Vec<ShapedDevice>,

  /// An LPM trie storing the IP mappings of all shaped devices,
  /// allowing for quick IP-to-circuit mapping.
  pub trie: ip_network_table::IpNetworkTable<usize>,
}

impl Default for ConfigShapedDevices {
  fn default() -> Self {
    Self {
      devices: Vec::new(),
      trie: ip_network_table::IpNetworkTable::<usize>::new(),
    }
  }
}

impl ConfigShapedDevices {
  /// The path to the current `ShapedDevices.csv` file, determined
  /// by acquiring the prefix from the `/etc/lqos.conf` configuration
  /// file.
  pub fn path() -> Result<PathBuf, ShapedDevicesError> {
    let cfg =
      crate::load_config().map_err(|_| ShapedDevicesError::ConfigLoadError)?;
    let base_path = Path::new(&cfg.lqos_directory);
    let full_path = base_path.join("ShapedDevices.csv");
    log::info!("ShapedDevices.csv path: {:?}", full_path);
    Ok(full_path)
  }

  /// Does ShapedDevices.csv exist?
  pub fn exists() -> bool {
    if let Ok(path) = Self::path() {
      path.exists()
    } else {
      false
    }
  }

  /// Loads `ShapedDevices.csv` and constructs a `ConfigShapedDevices`
  /// object containing the resulting data.
  pub fn load() -> Result<Self, ShapedDevicesError> {
    let final_path = ConfigShapedDevices::path()?;
    let reader = ReaderBuilder::new()
      .comment(Some(b'#'))
      .trim(csv::Trim::All)
      .from_path(final_path);
    if reader.is_err() {
      error!("Unable to read ShapedDevices.csv");
      return Err(ShapedDevicesError::OpenFail);
    }
    let mut reader = reader.unwrap();

    // Example: StringRecord(["1", "968 Circle St., Gurnee, IL 60031", "1", "Device 1", "", "", "192.168.101.2", "", "25", "5", "10000", "10000", ""])

    let mut devices = Vec::with_capacity(SUPPORTED_CUSTOMERS);
    for result in reader.records() {
      if let Ok(result) = result {
        let device = ShapedDevice::from_csv(&result);
        if let Ok(device) = device {
          devices.push(device);
        } else {
          log::error!("Error reading Device line: {:?}", &device);
          return Err(ShapedDevicesError::DeviceDecode(format!(
            "DEVICE DECODE: {device:?}"
          )));
        }
      } else {
        log::error!("Error reading CSV record: {:?}", result);
        if let csv::ErrorKind::UnequalLengths { pos, expected_len, len } =
          result.as_ref().err().as_ref().unwrap().kind()
        {
          if let Some(pos) = &pos {
            let msg = format!(
              "At line {}, position {}. Expected {} fields, found {}",
              pos.line(),
              pos.byte(),
              expected_len,
              len
            );
            error!("CSV decode error: {msg}");
            return Err(ShapedDevicesError::UnequalLengths(msg));
          } else {
            let msg = format!(
              "Unknown position. Expected {expected_len} fields, found {len}"
            );
            error!("CSV decode error: {msg}");
            return Err(ShapedDevicesError::UnequalLengths(msg));
          }
        }
        return Err(ShapedDevicesError::GenericCsvError(format!(
          "CSV FILE: {result:?}"
        )));
      }
    }
    let trie = ConfigShapedDevices::make_trie(&devices);
    Ok(Self { devices, trie })
  }

  /// Replace the current shaped devices list with a new one
  pub fn replace_with_new_data(&mut self, devices: Vec<ShapedDevice>) {
    self.devices = devices;
    log::info!("{:?}", self.devices);
    self.trie = ConfigShapedDevices::make_trie(&self.devices);
  }

  fn make_trie(
    devices: &[ShapedDevice],
  ) -> ip_network_table::IpNetworkTable<usize> {
    use ip_network::IpNetwork;
    let mut table = ip_network_table::IpNetworkTable::new();
    devices.iter().enumerate().map(|(i, d)| (i, d.to_ipv6_list())).for_each(
      |(id, ips)| {
        ips.iter().for_each(|(ip, cidr)| {
          if let Ok(net) = IpNetwork::new(*ip, (*cidr) as u8) {
            table.insert(net, id);
          }
        });
      },
    );
    table
  }

  fn to_csv_string(&self) -> Result<String, ShapedDevicesError> {
    let mut writer = WriterBuilder::new()
      .quote_style(QuoteStyle::NonNumeric)
      .from_writer(vec![]);
    for d in self.devices.iter().map(SerializableShapedDevice::from) {
      if writer.serialize(&d).is_err() {
        error!("Unable to serialize record, {:?}", d);
        return Err(ShapedDevicesError::SerializeFail);
      }
    }

    let data = String::from_utf8(
      writer.into_inner().map_err(|_| ShapedDevicesError::SerializeFail)?,
    )
    .map_err(|_| ShapedDevicesError::Utf8Error)?;
    Ok(data)
  }

  /// Saves the current shaped devices list to `ShapedDevices.csv`
  pub fn write_csv(&self, filename: &str) -> Result<(), ShapedDevicesError> {
    let cfg =
      crate::load_config().map_err(|_| ShapedDevicesError::ConfigLoadError)?;
    let base_path = Path::new(&cfg.lqos_directory);
    let path = base_path.join(filename);
    let csv = self.to_csv_string()?;
    if std::fs::write(path, csv).is_err() {
      error!("Unable to write ShapedDevices.csv. Permissions?");
      return Err(ShapedDevicesError::WriteFail);
    }
    //println!("Would write to file: {}", csv);
    Ok(())
  }
}

#[derive(Error, Debug)]
pub enum ShapedDevicesError {
  #[error("Error converting string to number in CSV record")]
  CsvEntryParseError(String),
  #[error("Unable to parse IPv4 address")]
  IPv4ParseError(String),
  #[error("Unable to parse IPv6 address")]
  IPv6ParseError(String),
  #[error("Unable to load /etc/lqos.conf")]
  ConfigLoadError,
  #[error("Unable to open/read ShapedDevices.csv")]
  OpenFail,
  #[error("Unable to write ShapedDevices.csv")]
  WriteFail,
  #[error("Unable to serialize - see next error for details")]
  SerializeFail,
  #[error("String does not translate to UTF-8")]
  Utf8Error,
  #[error("Unable to decode device entry in ShapedDevices.csv")]
  DeviceDecode(String),
  #[error("CSV line contains an unepected number of entries")]
  UnequalLengths(String),
  #[error("Unexpected CSV file error")]
  GenericCsvError(String),
}

#[cfg(test)]
mod test {
  use std::net::{Ipv4Addr, Ipv6Addr};

  use super::*;

  #[test]
  fn test_simple_ipv4_parse() {
    let (ip, cidr) = ShapedDevice::parse_cidr_v4("1.2.3.4").unwrap();
    assert_eq!(cidr, 32);
    assert_eq!("1.2.3.4".parse::<Ipv4Addr>().unwrap(), ip);
  }

  #[test]
  fn test_cidr_ipv4_parse() {
    let (ip, cidr) = ShapedDevice::parse_cidr_v4("1.2.3.4/24").unwrap();
    assert_eq!(cidr, 24);
    assert_eq!("1.2.3.4".parse::<Ipv4Addr>().unwrap(), ip);
  }

  #[test]
  fn test_bad_ipv4_parse() {
    let r = ShapedDevice::parse_cidr_v4("bad wolf");
    assert!(r.is_err());
  }

  #[test]
  fn test_nearly_ok_ipv4_parse() {
    let r = ShapedDevice::parse_cidr_v4("192.168.1.256/32");
    assert!(r.is_err());
  }

  #[test]
  fn test_single_ipv4() {
    let r = ShapedDevice::parse_ipv4("1.2.3.4");
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].0, "1.2.3.4".parse::<Ipv4Addr>().unwrap());
    assert_eq!(r[0].1, 32);
  }

  #[test]
  fn test_two_ipv4() {
    let r = ShapedDevice::parse_ipv4("1.2.3.4, 1.2.3.4/24");
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].0, "1.2.3.4".parse::<Ipv4Addr>().unwrap());
    assert_eq!(r[0].1, 32);
    assert_eq!(r[1].0, "1.2.3.4".parse::<Ipv4Addr>().unwrap());
    assert_eq!(r[1].1, 24);
  }

  #[test]
  fn test_simple_ipv6_parse() {
    let (ip, cidr) = ShapedDevice::parse_cidr_v6("fd77::1:5").unwrap();
    assert_eq!(cidr, 128);
    assert_eq!("fd77::1:5".parse::<Ipv6Addr>().unwrap(), ip);
  }

  #[test]
  fn test_cidr_ipv6_parse() {
    let (ip, cidr) = ShapedDevice::parse_cidr_v6("fd77::1:5/64").unwrap();
    assert_eq!(cidr, 64);
    assert_eq!("fd77::1:5".parse::<Ipv6Addr>().unwrap(), ip);
  }

  #[test]
  fn test_bad_ipv6_parse() {
    let r = ShapedDevice::parse_cidr_v6("bad wolf");
    assert!(r.is_err());
  }

  #[test]
  fn test_nearly_ok_ipv6_parse() {
    let r = ShapedDevice::parse_cidr_v6("fd77::1::5");
    assert!(r.is_err());
  }

  #[test]
  fn test_single_ipv6() {
    let r = ShapedDevice::parse_ipv6("fd77::1:5");
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].0, "fd77::1:5".parse::<Ipv6Addr>().unwrap());
    assert_eq!(r[0].1, 128);
  }

  #[test]
  fn test_two_ipv6() {
    let r = ShapedDevice::parse_ipv6("fd77::1:5, fd77::1:5/64");
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].0, "fd77::1:5".parse::<Ipv6Addr>().unwrap());
    assert_eq!(r[0].1, 128);
    assert_eq!(r[1].0, "fd77::1:5".parse::<Ipv6Addr>().unwrap());
    assert_eq!(r[1].1, 64);
  }

  #[test]
  fn build_and_test_simple_trie() {
    let devices = vec![
      ShapedDevice {
        circuit_id: "One".to_string(),
        ipv4: ShapedDevice::parse_ipv4("192.168.1.0/24"),
        ..Default::default()
      },
      ShapedDevice {
        circuit_id: "One".to_string(),
        ipv4: ShapedDevice::parse_ipv4("1.2.3.4"),
        ..Default::default()
      },
    ];
    let trie = ConfigShapedDevices::make_trie(&devices);
    assert_eq!(trie.len(), (0, 2));
    assert!(trie
      .longest_match(ShapedDevice::parse_cidr_v4("192.168.2.2").unwrap().0)
      .is_none());

    let addr: Ipv4Addr = "192.168.1.2".parse().unwrap();
    let v6 = addr.to_ipv6_mapped();
    assert!(trie.longest_match(v6).is_some());

    let addr: Ipv4Addr = "1.2.3.4".parse().unwrap();
    let v6 = addr.to_ipv6_mapped();
    assert!(trie.longest_match(v6).is_some());
  }
}
