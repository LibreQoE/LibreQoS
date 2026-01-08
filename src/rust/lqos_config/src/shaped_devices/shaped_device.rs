use super::ShapedDevicesError;
use allocative::Allocative;
use csv::StringRecord;
use lqos_utils::hash_to_i64;
use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, Ipv6Addr};
use tracing::error;

/// Represents a row in the `ShapedDevices.csv` file.
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Allocative)]
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
    pub download_min_mbps: f32,

    /// Minimum upload: this is the bandwidth level the shaper will try to
    /// ensure is always available.
    pub upload_min_mbps: f32,

    /// Maximum download speed, when possible.
    pub download_max_mbps: f32,

    /// Maximum upload speed when possible.
    pub upload_max_mbps: f32,

    /// Generic comments field, does nothing.
    pub comment: String,

    /// Optional per-circuit SQM override token. Accepts "cake", "fq_codel",
    /// "none", or directional "down_sqm/up_sqm" values like "cake/none" or
    /// "/fq_codel". A single token applies to both directions; empty means
    /// "use global default".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sqm_override: Option<String>,

    /// Hash of the circuit ID, used for internal lookups.
    #[serde(skip)]
    pub circuit_hash: i64,

    /// Hash of the device ID, used for internal lookups.
    #[serde(skip)]
    pub device_hash: i64,

    /// Hash of the parent node, used for internal lookups.
    #[serde(skip)]
    pub parent_hash: i64,
}

impl ShapedDevice {
    /// Creates a new `ShapedDevice` instance from a CSV string record.
    ///
    /// This function parses a CSV record containing device configuration data and constructs
    /// a `ShapedDevice` with all necessary fields populated. The CSV record must contain
    /// exactly 13 fields in the following order (optionally a 14th `sqm` field may be present):
    ///
    /// 1. Circuit ID
    /// 2. Circuit Name
    /// 3. Device ID
    /// 4. Device Name
    /// 5. Parent Node
    /// 6. MAC Address
    /// 7. IPv4 Addresses (comma-separated, CIDR notation supported)
    /// 8. IPv6 Addresses (comma-separated, CIDR notation supported)
    /// 9. Download Min Mbps
    /// 10. Upload Min Mbps
    /// 11. Download Max Mbps
    /// 12. Upload Max Mbps
    /// 13. Comment
    /// 14. sqm (optional; allowed values: "cake", "fq_codel", "none", or
    ///     a directional override in the form "down_sqm/up_sqm". Either side
    ///     may be empty to indicate no override for that direction, e.g.
    ///     "cake/" or "/fq_codel".)
    ///
    /// # Arguments
    ///
    /// * `record` - A reference to a CSV `StringRecord` containing the device data
    ///
    /// # Returns
    ///
    /// * `Ok(ShapedDevice)` - Successfully parsed device configuration
    /// * `Err(ShapedDevicesError)` - If parsing fails due to invalid data format
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// * The bandwidth values (min/max upload/download) cannot be parsed as unsigned integers
    /// * The CSV record doesn't contain the expected number of fields
    pub fn from_csv(record: &StringRecord) -> Result<Self, ShapedDevicesError> {
        // Parse mandatory fields (first 13 entries)
        let mut device = Self {
            circuit_id: record[0].to_string(),
            circuit_name: record[1].to_string(),
            device_id: record[2].to_string(),
            device_name: record[3].to_string(),
            parent_node: record[4].to_string(),
            mac: record[5].to_string(),
            ipv4: ShapedDevice::parse_ipv4(&record[6]),
            ipv6: ShapedDevice::parse_ipv6(&record[7]),
            download_min_mbps: {
                let rate = record[8]
                    .parse::<f32>()
                    .map_err(|_| ShapedDevicesError::CsvEntryParseError(record[8].to_string()))?;
                if rate < 0.01 {
                    return Err(ShapedDevicesError::CsvEntryParseError(format!(
                        "Download min rate {} too small (minimum 0.01 Mbps)",
                        rate
                    )));
                }
                rate
            },
            upload_min_mbps: {
                let rate = record[9]
                    .parse::<f32>()
                    .map_err(|_| ShapedDevicesError::CsvEntryParseError(record[9].to_string()))?;
                if rate < 0.01 {
                    return Err(ShapedDevicesError::CsvEntryParseError(format!(
                        "Upload min rate {} too small (minimum 0.01 Mbps)",
                        rate
                    )));
                }
                rate
            },
            download_max_mbps: {
                let rate = record[10]
                    .parse::<f32>()
                    .map_err(|_| ShapedDevicesError::CsvEntryParseError(record[10].to_string()))?;
                if rate < 0.01 {
                    return Err(ShapedDevicesError::CsvEntryParseError(format!(
                        "Download max rate {} too small (minimum 0.01 Mbps)",
                        rate
                    )));
                }
                rate
            },
            upload_max_mbps: {
                let rate = record[11]
                    .parse::<f32>()
                    .map_err(|_| ShapedDevicesError::CsvEntryParseError(record[11].to_string()))?;
                if rate < 0.01 {
                    return Err(ShapedDevicesError::CsvEntryParseError(format!(
                        "Upload max rate {} too small (minimum 0.01 Mbps)",
                        rate
                    )));
                }
                rate
            },
            comment: record[12].to_string(),
            sqm_override: None,
            circuit_hash: hash_to_i64(&record[0]),
            device_hash: hash_to_i64(&record[2]),
            parent_hash: hash_to_i64(&record[4]),
        };

        // Optional 14th field: per-circuit SQM override token
        if record.len() >= 14 {
            let raw = record[13].trim();
            if !raw.is_empty() {
                // Normalize case and whitespace around optional '/'
                let token = raw.to_lowercase();
                if token.contains('/') {
                    // Directional override: down_sqm/up_sqm (either may be empty)
                    let mut parts = token.splitn(2, '/');
                    let down = parts.next().unwrap_or("").trim();
                    let up = parts.next().unwrap_or("").trim();

                    // Validate each side if present
                    let valid =
                        |s: &str| -> bool { matches!(s, "" | "cake" | "fq_codel" | "none") };
                    if !valid(down) || !valid(up) {
                        return Err(ShapedDevicesError::CsvEntryParseError(format!(
                            "Invalid directional sqm override '{token}'. Allowed: 'cake', 'fq_codel', 'none', or 'down_sqm/up_sqm' (e.g. 'cake/fq_codel', '/none')"
                        )));
                    }

                    // Store normalized (trimmed, lowercase) representation exactly as down_sqm/up_sqm
                    device.sqm_override = Some(format!("{down}/{up}"));
                } else {
                    // Single token applies to both directions when used
                    match token.as_str() {
                        "cake" | "fq_codel" | "none" => device.sqm_override = Some(token),
                        other => {
                            return Err(ShapedDevicesError::CsvEntryParseError(format!(
                                "Invalid sqm override '{other}'. Allowed values: 'cake', 'fq_codel', 'none', or 'down_sqm/up_sqm' (e.g. 'cake/fq_codel', '/none')"
                            )));
                        }
                    }
                }
            }
        }

        Ok(device)
    }

    pub(crate) fn parse_cidr_v4(address: &str) -> Result<(Ipv4Addr, u32), ShapedDevicesError> {
        if address.contains('/') {
            let split: Vec<&str> = address.split('/').collect();
            if split.len() != 2 {
                error!("Unable to parse IPv4 {address}");
                return Err(ShapedDevicesError::IPv4ParseError(address.to_string()));
            }
            Ok((
                split[0]
                    .parse()
                    .map_err(|_| ShapedDevicesError::IPv4ParseError(address.to_string()))?,
                split[1]
                    .parse()
                    .map_err(|_| ShapedDevicesError::IPv4ParseError(address.to_string()))?,
            ))
        } else {
            Ok((
                address
                    .parse()
                    .map_err(|_| ShapedDevicesError::IPv4ParseError(address.to_string()))?,
                32,
            ))
        }
    }

    pub(crate) fn parse_ipv4(str: &str) -> Vec<(Ipv4Addr, u32)> {
        let mut result = Vec::new();
        if str.contains(',') {
            for ip in str.split(',') {
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

    pub(crate) fn parse_cidr_v6(address: &str) -> Result<(Ipv6Addr, u32), ShapedDevicesError> {
        if address.contains('/') {
            let split: Vec<&str> = address.split('/').collect();
            if split.len() != 2 {
                error!("Unable to parse IPv6: {address}");
                return Err(ShapedDevicesError::IPv6ParseError(address.to_string()));
            }
            Ok((
                split[0]
                    .parse()
                    .map_err(|_| ShapedDevicesError::IPv6ParseError(address.to_string()))?,
                split[1]
                    .parse()
                    .map_err(|_| ShapedDevicesError::IPv6ParseError(address.to_string()))?,
            ))
        } else {
            Ok((
                address
                    .parse()
                    .map_err(|_| ShapedDevicesError::IPv6ParseError(address.to_string()))?,
                128,
            ))
        }
    }

    pub(crate) fn parse_ipv6(str: &str) -> Vec<(Ipv6Addr, u32)> {
        let mut result = Vec::new();
        if str.contains(',') {
            for ip in str.split(',') {
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

#[cfg(test)]
mod tests {
    use super::*;
    use csv::StringRecord;

    #[test]
    fn test_fractional_rate_parsing() {
        // Test parsing fractional rates
        let record = StringRecord::from(vec![
            "test1",
            "Test Circuit",
            "device1",
            "Test Device",
            "site1",
            "00:00:00:00:00:01",
            "192.168.1.1",
            "",
            "0.5",
            "1.0",
            "2.5",
            "3.0",
            "Test fractional rates",
        ]);

        let device = ShapedDevice::from_csv(&record).expect("Should parse fractional rates");

        assert_eq!(device.download_min_mbps, 0.5);
        assert_eq!(device.upload_min_mbps, 1.0);
        assert_eq!(device.download_max_mbps, 2.5);
        assert_eq!(device.upload_max_mbps, 3.0);
    }

    #[test]
    fn test_integer_rate_parsing() {
        // Test parsing integer rates (backward compatibility)
        let record = StringRecord::from(vec![
            "test2",
            "Test Circuit 2",
            "device2",
            "Test Device 2",
            "site2",
            "00:00:00:00:00:02",
            "192.168.1.2",
            "",
            "10",
            "20",
            "100",
            "200",
            "Integer rates",
        ]);

        let device = ShapedDevice::from_csv(&record).expect("Should parse integer rates");

        assert_eq!(device.download_min_mbps, 10.0);
        assert_eq!(device.upload_min_mbps, 20.0);
        assert_eq!(device.download_max_mbps, 100.0);
        assert_eq!(device.upload_max_mbps, 200.0);
    }

    #[test]
    fn test_rate_validation_too_small() {
        // Test that rates below 0.01 are rejected
        let record = StringRecord::from(vec![
            "test3",
            "Test Circuit 3",
            "device3",
            "Test Device 3",
            "site3",
            "00:00:00:00:00:03",
            "192.168.1.3",
            "",
            "0.001",
            "1.0",
            "2.5",
            "3.0",
            "Rate too small",
        ]);

        let result = ShapedDevice::from_csv(&record);
        assert!(result.is_err(), "Should reject rates below 0.01 Mbps");
    }

    #[test]
    fn test_invalid_rate_parsing() {
        // Test that invalid rate strings are rejected
        let record = StringRecord::from(vec![
            "test4",
            "Test Circuit 4",
            "device4",
            "Test Device 4",
            "site4",
            "00:00:00:00:00:04",
            "192.168.1.4",
            "",
            "invalid",
            "1.0",
            "2.5",
            "3.0",
            "Invalid rate",
        ]);

        let result = ShapedDevice::from_csv(&record);
        assert!(result.is_err(), "Should reject invalid rate strings");
    }
}
