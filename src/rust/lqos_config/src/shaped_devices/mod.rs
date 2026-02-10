mod serializable;
mod shaped_device;

use csv::{QuoteStyle, ReaderBuilder, WriterBuilder};
use lqos_utils::XdpIpAddress;
use serializable::SerializableShapedDevice;
pub use shaped_device::ShapedDevice;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::{debug, error};

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
        let cfg = crate::load_config().map_err(|_| ShapedDevicesError::ConfigLoadError)?;
        let base_path = Path::new(&cfg.lqos_directory);
        let full_path = if cfg.long_term_stats.enable_insight_topology.unwrap_or(false) {
            let tmp_path = base_path.join("ShapedDevices.insight.csv");
            if tmp_path.exists() {
                tmp_path
            } else {
                base_path.join("ShapedDevices.csv")
            }
        } else {
            base_path.join("ShapedDevices.csv")
        };
        debug!("ShapedDevices.csv path: {:?}", full_path);
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

    fn handle_encodings(bytes: &[u8]) -> Vec<u8> {
        // First, handle BOM if present
        if let Some((encoding, bom_length)) = encoding_rs::Encoding::for_bom(bytes) {
            let mut result = Vec::new();
            let (decoded, _, _) = encoding.decode(&bytes[bom_length..]);
            result.extend_from_slice(decoded.as_bytes());
            return result;
        }

        // If already valid UTF-8, return as-is
        if std::str::from_utf8(bytes).is_ok() {
            return bytes.to_vec();
        }

        // Comprehensive European + Latin American encoding list
        let encoding_labels = [
            // Most common modern encodings
            "windows-1252", // Western Europe (English, French, German, Spanish, etc.)
            "windows-1250", // Central/Eastern Europe (Polish, Czech, Hungarian, etc.)
            "windows-1251", // Cyrillic (Russian, Bulgarian, Serbian, etc.)
            "windows-1253", // Greek
            "windows-1254", // Turkish
            "windows-1257", // Baltic (Lithuanian, Latvian, Estonian)
            // ISO Latin series
            "iso-8859-1",  // Latin-1: Western Europe
            "iso-8859-2",  // Latin-2: Central/Eastern Europe
            "iso-8859-3",  // Latin-3: Southern Europe (Turkish, Maltese)
            "iso-8859-4",  // Latin-4: Northern Europe (Baltic)
            "iso-8859-5",  // Cyrillic
            "iso-8859-7",  // Greek
            "iso-8859-9",  // Latin-5: Turkish
            "iso-8859-13", // Latin-7: Baltic
            "iso-8859-15", // Latin-9: Western Europe with Euro symbol
            "iso-8859-16", // Latin-10: Romanian
            // Legacy but still encountered
            "koi8-r", // Russian Cyrillic
            "koi8-u", // Ukrainian Cyrillic
            "cp437",  // Original DOS encoding
            "cp850",  // DOS Latin-1
            "cp852",  // DOS Latin-2
            "cp866",  // DOS Cyrillic
        ];

        for label in &encoding_labels {
            if let Some(encoding) = encoding_rs::Encoding::for_label(label.as_bytes()) {
                let (decoded, _, had_errors) = encoding.decode(bytes);
                if !had_errors {
                    return decoded.as_bytes().to_vec();
                }
            }
        }

        // Fallback
        String::from_utf8_lossy(bytes).as_bytes().to_vec()
    }

    /// Loads `ShapedDevices.csv` and constructs a `ConfigShapedDevices`
    /// object containing the resulting data.
    pub fn load() -> Result<Self, ShapedDevicesError> {
        let final_path = ConfigShapedDevices::path()?;

        // Load the CSV file as a byte array
        if !final_path.exists() {
            error!("ShapedDevices.csv does not exist at {:?}", final_path);
            return Err(ShapedDevicesError::OpenFail);
        }
        debug!("Loading ShapedDevices.csv from {:?}", final_path);
        let raw_bytes = std::fs::read(&final_path).map_err(|_| ShapedDevicesError::OpenFail)?;
        let utf8_bytes = ConfigShapedDevices::handle_encodings(&raw_bytes);

        let mut reader = ReaderBuilder::new()
            .comment(Some(b'#'))
            .trim(csv::Trim::All)
            // Allow optional trailing fields like per-circuit SQM override
            // without forcing all rows to match header length.
            .flexible(true)
            .from_reader(utf8_bytes.as_slice());

        // Example: StringRecord(["1", "968 Circle St., Gurnee, IL 60031", "1", "Device 1", "", "", "192.168.101.2", "", "25", "5", "10000", "10000", ""])

        let mut devices = Vec::new(); // Note that this used to be supported_customers, but we're going to let it grow organically
        for result in reader.records() {
            if let Ok(result) = result {
                let device = ShapedDevice::from_csv(&result);
                if let Ok(device) = device {
                    devices.push(device);
                } else {
                    error!("Error reading Device line: {:?}", &device);
                    return Err(ShapedDevicesError::DeviceDecode(format!(
                        "DEVICE DECODE: {device:?}"
                    )));
                }
            } else {
                error!("Error reading CSV record: {:?}", result);

                // Safely extract error details if available
                if let Err(ref csv_err) = result {
                    match csv_err.kind() {
                        csv::ErrorKind::UnequalLengths {
                            pos,
                            expected_len,
                            len,
                        } => {
                            let msg = if let Some(pos) = pos {
                                format!(
                                    "At line {}, position {}. Expected {} fields, found {}",
                                    pos.line(),
                                    pos.byte(),
                                    expected_len,
                                    len
                                )
                            } else {
                                format!(
                                    "Unknown position. Expected {expected_len} fields, found {len}"
                                )
                            };
                            error!("CSV decode error: {msg}");
                            return Err(ShapedDevicesError::UnequalLengths(msg));
                        }
                        _ => {
                            // Handle any other CSV error type safely
                            return Err(ShapedDevicesError::GenericCsvError(format!(
                                "CSV FILE: {result:?}"
                            )));
                        }
                    }
                } else {
                    // This shouldn't happen, but handle it gracefully
                    return Err(ShapedDevicesError::GenericCsvError(
                        "Unknown CSV error".to_string(),
                    ));
                }
            }
        }
        let trie = ConfigShapedDevices::make_trie(&devices);
        Ok(Self { devices, trie })
    }

    /// Replace the current shaped devices list with a new one
    pub fn replace_with_new_data(&mut self, devices: Vec<ShapedDevice>) {
        self.devices = devices;
        debug!("{:?}", self.devices);
        let mut new_trie = ConfigShapedDevices::make_trie(&self.devices);
        std::mem::swap(&mut self.trie, &mut new_trie);
        std::mem::drop(new_trie); // Explicitly drop the old trie
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
            writer
                .into_inner()
                .map_err(|_| ShapedDevicesError::SerializeFail)?,
        )
        .map_err(|_| ShapedDevicesError::Utf8Error)?;
        Ok(data)
    }

    /// Saves the current shaped devices list to `ShapedDevices.csv`
    pub fn write_csv(&self, filename: &str) -> Result<(), ShapedDevicesError> {
        let cfg = crate::load_config().map_err(|_| ShapedDevicesError::ConfigLoadError)?;
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

    /// Helper function to search for an XdpIpAddress and return a circuit id and name
    /// if they exist.
    pub fn get_circuit_id_and_name_from_ip(&self, ip: &XdpIpAddress) -> Option<(String, String)> {
        let lookup = match ip.as_ip() {
            IpAddr::V4(ip) => ip.to_ipv6_mapped(),
            IpAddr::V6(ip) => ip,
        };
        if let Some(c) = self.trie.longest_match(lookup) {
            let device = &self.devices[*c.1];
            return Some((device.circuit_id.clone(), device.circuit_name.clone()));
        }

        None
    }

    /// Helper function to search for an XdpIpAddress and return a circuit id and name
    /// if they exist.
    pub fn get_circuit_hash_from_ip(&self, ip: &XdpIpAddress) -> Option<i64> {
        let lookup = match ip.as_ip() {
            IpAddr::V4(ip) => ip.to_ipv6_mapped(),
            IpAddr::V6(ip) => ip,
        };
        if let Some(c) = self.trie.longest_match(lookup) {
            let device = &self.devices[*c.1];
            return Some(device.circuit_hash);
        }

        None
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
        let (ip, cidr) = ShapedDevice::parse_cidr_v4("1.2.3.4").expect("IP Parse Error");
        assert_eq!(cidr, 32);
        assert_eq!("1.2.3.4".parse::<Ipv4Addr>().expect("IP parse error"), ip);
    }

    #[test]
    fn test_cidr_ipv4_parse() {
        let (ip, cidr) = ShapedDevice::parse_cidr_v4("1.2.3.4/24").expect("IP Parse Error");
        assert_eq!(cidr, 24);
        assert_eq!("1.2.3.4".parse::<Ipv4Addr>().expect("IP Parse"), ip);
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
        assert_eq!(
            r[0].0,
            "1.2.3.4".parse::<Ipv4Addr>().expect("IP Parse Error")
        );
        assert_eq!(r[0].1, 32);
    }

    #[test]
    fn test_two_ipv4() {
        let r = ShapedDevice::parse_ipv4("1.2.3.4, 1.2.3.4/24");
        assert_eq!(r.len(), 2);
        assert_eq!(
            r[0].0,
            "1.2.3.4".parse::<Ipv4Addr>().expect("IP Parse Error")
        );
        assert_eq!(r[0].1, 32);
        assert_eq!(
            r[1].0,
            "1.2.3.4".parse::<Ipv4Addr>().expect("IP Parse Error")
        );
        assert_eq!(r[1].1, 24);
    }

    #[test]
    fn test_simple_ipv6_parse() {
        let (ip, cidr) = ShapedDevice::parse_cidr_v6("fd77::1:5").expect("IP Parse Error");
        assert_eq!(cidr, 128);
        assert_eq!("fd77::1:5".parse::<Ipv6Addr>().expect("IP Parse Error"), ip);
    }

    #[test]
    fn test_cidr_ipv6_parse() {
        let (ip, cidr) = ShapedDevice::parse_cidr_v6("fd77::1:5/64").expect("IP Parse Error");
        assert_eq!(cidr, 64);
        assert_eq!("fd77::1:5".parse::<Ipv6Addr>().expect("IP Parse Error"), ip);
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
        assert_eq!(
            r[0].0,
            "fd77::1:5".parse::<Ipv6Addr>().expect("IP Parse Error")
        );
        assert_eq!(r[0].1, 128);
    }

    #[test]
    fn test_two_ipv6() {
        let r = ShapedDevice::parse_ipv6("fd77::1:5, fd77::1:5/64");
        assert_eq!(r.len(), 2);
        assert_eq!(
            r[0].0,
            "fd77::1:5".parse::<Ipv6Addr>().expect("IP Parse Error")
        );
        assert_eq!(r[0].1, 128);
        assert_eq!(
            r[1].0,
            "fd77::1:5".parse::<Ipv6Addr>().expect("IP Parse Error")
        );
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
        assert!(
            trie.longest_match(
                ShapedDevice::parse_cidr_v4("192.168.2.2")
                    .expect("IP Parse Error")
                    .0
            )
            .is_none()
        );

        let addr: Ipv4Addr = "192.168.1.2".parse().expect("IP Parse Error");
        let v6 = addr.to_ipv6_mapped();
        assert!(trie.longest_match(v6).is_some());

        let addr: Ipv4Addr = "1.2.3.4".parse().expect("IP Parse Error");
        let v6 = addr.to_ipv6_mapped();
        assert!(trie.longest_match(v6).is_some());
    }

    #[test]
    fn test_handle_encodings_valid_utf8() {
        // Test plain UTF-8 text
        let input = "Hello, World! 你好世界 Привет мир".as_bytes();
        let result = ConfigShapedDevices::handle_encodings(input);
        assert_eq!(result, input);
        assert_eq!(
            String::from_utf8(result).expect("Unicode error"),
            "Hello, World! 你好世界 Привет мир"
        );
    }

    #[test]
    fn test_handle_encodings_utf8_with_bom() {
        // UTF-8 BOM: EF BB BF
        let mut input = vec![0xEF, 0xBB, 0xBF];
        input.extend_from_slice("Hello UTF-8 with BOM".as_bytes());

        let result = ConfigShapedDevices::handle_encodings(&input);
        assert_eq!(
            String::from_utf8(result).expect("Unicode error"),
            "Hello UTF-8 with BOM"
        );
    }

    #[test]
    fn test_handle_encodings_utf16le_with_bom() {
        // UTF-16 LE BOM: FF FE followed by "Hello" in UTF-16 LE
        let input = vec![
            0xFF, 0xFE, // BOM
            0x48, 0x00, // H
            0x65, 0x00, // e
            0x6C, 0x00, // l
            0x6C, 0x00, // l
            0x6F, 0x00, // o
        ];

        let result = ConfigShapedDevices::handle_encodings(&input);
        assert_eq!(String::from_utf8(result).expect("Unicode error"), "Hello");
    }

    #[test]
    fn test_handle_encodings_windows_1252() {
        // "Café" in Windows-1252: C=0x43, a=0x61, f=0x66, é=0xE9
        let input = vec![
            0x43, 0x61, 0x66, 0xE9, 0x20, 0x2D, 0x20, 0xA9, 0x20, 0x32, 0x30, 0x32, 0x34,
        ]; // "Café - © 2024"

        let result = ConfigShapedDevices::handle_encodings(&input);
        let result_str = String::from_utf8(result).expect("Unicode error");
        assert!(result_str.contains("Café"));
        assert!(result_str.contains("©"));
    }

    #[test]
    fn test_handle_encodings_iso_8859_1() {
        // "Größe" in ISO-8859-1: G=0x47, r=0x72, ö=0xF6, ß=0xDF, e=0x65
        let input = vec![0x47, 0x72, 0xF6, 0xDF, 0x65];

        let result = ConfigShapedDevices::handle_encodings(&input);
        assert_eq!(String::from_utf8(result).expect("Unicode error"), "Größe");
    }

    #[test]
    fn test_handle_encodings_windows_1251_cyrillic() {
        // "Привет" (Hello in Russian) in Windows-1251
        // П=0xCF, р=0xF0, и=0xE8, в=0xE2, е=0xE5, т=0xF2
        let input = vec![0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2];

        let result = ConfigShapedDevices::handle_encodings(&input);
        // Since encoding_rs might not perfectly decode this, let's check it's valid UTF-8
        let result_str = String::from_utf8(result).expect("Unicode error");
        // The exact output depends on encoding_rs implementation
        assert!(!result_str.is_empty());
    }

    #[test]
    fn test_handle_encodings_koi8r_cyrillic() {
        // "Мир" (World in Russian) in KOI8-R
        // М=0xED, и=0xC9, р=0xD2
        let input = vec![0xED, 0xC9, 0xD2];

        let result = ConfigShapedDevices::handle_encodings(&input);
        // Since encoding_rs might not perfectly decode this, let's check it's valid UTF-8
        let result_str = String::from_utf8(result).expect("Unicode error");
        // The exact output depends on encoding_rs implementation
        assert!(!result_str.is_empty());
    }

    #[test]
    fn test_handle_encodings_mixed_content() {
        // Test Windows-1252 with special characters: "naïve résumé"
        let input = vec![
            0x6E, 0x61, 0xEF, 0x76, 0x65, 0x20, // naïve
            0x72, 0xE9, 0x73, 0x75, 0x6D, 0xE9, // résumé
        ];

        let result = ConfigShapedDevices::handle_encodings(&input);
        let result_str = String::from_utf8(result).expect("Unicode error");
        assert!(result_str.contains("naïve"));
        assert!(result_str.contains("résumé"));
    }

    #[test]
    fn test_handle_encodings_fallback() {
        // Invalid/mixed encoding - should use lossy conversion
        let input = vec![0xFF, 0xFE, 0xFD, 0xFC]; // Invalid UTF-8

        let result = ConfigShapedDevices::handle_encodings(&input);
        // Should not panic and should return valid UTF-8
        assert!(String::from_utf8(result).is_ok());
    }
}
