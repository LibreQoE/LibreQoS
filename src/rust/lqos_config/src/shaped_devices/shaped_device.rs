use std::net::{Ipv4Addr, Ipv6Addr};
use anyhow::{Result, Error};
use csv::StringRecord;
use serde::{Serialize, Deserialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShapedDevice {
    // Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,MAC,IPv4,IPv6,Download Min Mbps,Upload Min Mbps,Download Max Mbps,Upload Max Mbps,Comment
    pub circuit_id: String,
    pub circuit_name: String,
    pub device_id: String,
    pub device_name: String,
    pub parent_node: String,
    pub mac: String,
    pub ipv4: Vec<(Ipv4Addr, u32)>,
    pub ipv6: Vec<(Ipv6Addr, u32)>,
    pub download_min_mbps: u32,
    pub upload_min_mbps: u32,
    pub download_max_mbps: u32,
    pub upload_max_mbps: u32,
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
    pub(crate) fn from_csv(record: &StringRecord) -> Result<Self> {
        Ok(Self {
            circuit_id: record[0].to_string(),
            circuit_name: record[1].to_string(),
            device_id: record[2].to_string(),
            device_name: record[3].to_string(),
            parent_node: record[4].to_string(),
            mac: record[5].to_string(),
            ipv4: ShapedDevice::parse_ipv4(&record[6]),
            ipv6: ShapedDevice::parse_ipv6(&record[7]),
            download_min_mbps: record[8].parse()?,
            upload_min_mbps: record[9].parse()?,
            download_max_mbps: record[10].parse()?,
            upload_max_mbps: record[11].parse()?,
            comment: record[12].to_string(),
        })
    }

    pub(crate) fn parse_cidr_v4(address: &str) -> Result<(Ipv4Addr, u32)> {
        if address.contains("/") {
            let split : Vec<&str> = address.split("/").collect();
            if split.len() != 2 {
                return Err(Error::msg("Unable to parse IPv4"));
            }
            return Ok((
                split[0].parse()?,
                split[1].parse()?
            ))
        } else {
            return Ok((
                address.parse()?,
                32
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

    pub(crate) fn parse_cidr_v6(address: &str) -> Result<(Ipv6Addr, u32)> {
        if address.contains("/") {
            let split : Vec<&str> = address.split("/").collect();
            if split.len() != 2 {
                return Err(Error::msg("Unable to parse IPv6"));
            }
            return Ok((
                split[0].parse()?,
                split[1].parse()?
            ))
        } else {
            return Ok((
                address.parse()?,
                128
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
            result.push((
                ipv4.to_ipv6_mapped(),
                cidr + 96
            ));
        }
        result.extend_from_slice(&self.ipv6);

        result
    }
}

