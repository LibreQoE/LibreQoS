mod serializable;
mod shaped_device;
use crate::{etc, SUPPORTED_CUSTOMERS};
use anyhow::Result;
use csv::{QuoteStyle, WriterBuilder, ReaderBuilder};
use serializable::SerializableShapedDevice;
pub use shaped_device::ShapedDevice;
use std::path::{Path, PathBuf};

/// Provides handling of the `ShapedDevices.csv` file that maps
/// circuits to traffic shaping.
pub struct ConfigShapedDevices {
    /// List of all devices subject to traffic shaping.
    pub devices: Vec<ShapedDevice>,

    /// An LPM trie storing the IP mappings of all shaped devices,
    /// allowing for quick IP-to-circuit mapping.
    pub trie: ip_network_table::IpNetworkTable<usize>,
}

impl ConfigShapedDevices {
    /// The path to the current `ShapedDevices.csv` file, determined
    /// by acquiring the prefix from the `/etc/lqos.conf` configuration
    /// file.
    pub fn path() -> Result<PathBuf> {
        let cfg = etc::EtcLqos::load()?;
        let base_path = Path::new(&cfg.lqos_directory);
        Ok(base_path.join("ShapedDevices.csv"))
    }

    /// Loads `ShapedDevices.csv` and constructs a `ConfigShapedDevices`
    /// object containing the resulting data.
    pub fn load() -> Result<Self> {
        let final_path = ConfigShapedDevices::path()?;
        let mut reader = ReaderBuilder::new()
            .comment(Some(b'#'))
            .from_path(final_path)?;

        // Example: StringRecord(["1", "968 Circle St., Gurnee, IL 60031", "1", "Device 1", "", "", "192.168.101.2", "", "25", "5", "10000", "10000", ""])
        
        let mut devices = Vec::with_capacity(SUPPORTED_CUSTOMERS);
        for result in reader.records() {            
            if let Ok(result) = result {
                let device = ShapedDevice::from_csv(&result);
                if let Ok(device) = device {
                    devices.push(device);
                } else {
                    log::error!("Error reading Device line: {:?}", &device);
                    return Err(anyhow::Error::msg(format!("DEVICE DECODE: {:?}", device)));
                }
            } else {
                log::error!("Error reading CSV record: {:?}", result);
                match result.as_ref().err().as_ref().unwrap().kind() {
                    csv::ErrorKind::UnequalLengths{ pos, expected_len, len} => {
                        if let Some(pos) = &pos {
                            let msg = format!("At line {}, position {}. Expected {} fields, found {}", pos.line(), pos.byte(), expected_len, len);
                            return Err(anyhow::Error::msg(msg));
                        } else {
                            let msg = format!("Unknown position. Expected {} fields, found {}", expected_len, len);
                            return Err(anyhow::Error::msg(msg));
                        }
                    }
                    _ => {}
                }
                return Err(anyhow::Error::msg(format!("CSV FILE: {:?}", result)));
            }
        }
        let trie = ConfigShapedDevices::make_trie(&devices);
        Ok(Self { devices, trie })
    }

    fn make_trie(devices: &[ShapedDevice]) -> ip_network_table::IpNetworkTable<usize> {
        use ip_network::IpNetwork;
        let mut table = ip_network_table::IpNetworkTable::new();
        devices
            .iter()
            .enumerate()
            .map(|(i, d)| (i, d.to_ipv6_list()))
            .for_each(|(id, ips)| {
                ips.iter().for_each(|(ip, cidr)| {
                    if let Ok(net) = IpNetwork::new(*ip, (*cidr) as u8) {
                        table.insert(net, id);
                    }
                });
            });
        table
    }

    fn to_csv_string(&self) -> Result<String> {
        let mut writer = WriterBuilder::new()
            .quote_style(QuoteStyle::NonNumeric)
            .from_writer(vec![]);
        for d in self
            .devices
            .iter()
            .map(|d| SerializableShapedDevice::from(d))
        {
            writer.serialize(d)?;
        }

        let data = String::from_utf8(writer.into_inner()?)?;
        Ok(data)
    }

    /// Saves the current shaped devices list to `ShapedDevices.csv`
    pub fn write_csv(&self, filename: &str) -> Result<()> {
        let cfg = etc::EtcLqos::load()?;
        let base_path = Path::new(&cfg.lqos_directory);
        let path = base_path.join(filename);
        let csv = self.to_csv_string()?;
        std::fs::write(path, csv)?;
        Ok(())
    }
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
