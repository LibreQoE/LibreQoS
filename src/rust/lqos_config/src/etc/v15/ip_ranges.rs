use allocative::Allocative;
use ip_network::IpNetwork;
use ip_network_table::IpNetworkTable;
use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, Ipv6Addr};

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Allocative)]
pub struct IpRanges {
    pub ignore_subnets: Vec<String>,
    pub allow_subnets: Vec<String>,
    pub unknown_ip_honors_ignore: Option<bool>,
    pub unknown_ip_honors_allow: Option<bool>,
}

impl Default for IpRanges {
    fn default() -> Self {
        Self {
            ignore_subnets: vec![],
            allow_subnets: vec![
                "172.16.0.0/12".to_string(),
                "10.0.0.0/8".to_string(),
                "100.64.0.0/10".to_string(),
                "192.168.0.0/16".to_string(),
            ],
            unknown_ip_honors_ignore: Some(true),
            unknown_ip_honors_allow: Some(true),
        }
    }
}

impl IpRanges {
    /// Maps the ignored IP ranges to an LPM table.
    pub fn ignored_network_table(&self) -> Result<IpNetworkTable<bool>, IpRangeError> {
        let mut ignored = IpNetworkTable::new();
        for excluded_ip in self.ignore_subnets.iter() {
            let split: Vec<_> = excluded_ip.split('/').collect();
            if split[0].contains(':') {
                // It's IPv6
                let ip_network: Ipv6Addr = split[0]
                    .parse()
                    .map_err(|e| IpRangeError::IpParseError { e: Box::new(e) })?;
                let ip = IpNetwork::new(
                    ip_network,
                    split[1].parse().map_err(|_| IpRangeError::InvalidNetmask)?,
                )
                .map_err(|e| IpRangeError::InvalidNetwork { e: Box::new(e) })?;
                ignored.insert(ip, true);
            } else {
                // It's IPv4
                let ip_network: Ipv4Addr = split[0]
                    .parse()
                    .map_err(|e| IpRangeError::IpParseError { e: Box::new(e) })?;
                let ip = IpNetwork::new(
                    ip_network,
                    split[1].parse().map_err(|_| IpRangeError::InvalidNetmask)?,
                )
                .map_err(|e| IpRangeError::InvalidNetwork { e: Box::new(e) })?;
                ignored.insert(ip, true);
            }
        }
        Ok(ignored)
    }

    /// Maps the allowed IP ranges to an LPM table.
    pub fn allowed_network_table(&self) -> Result<IpNetworkTable<bool>, IpRangeError> {
        let mut allowed = IpNetworkTable::new();
        for allowed_ip in self.allow_subnets.iter() {
            let split: Vec<_> = allowed_ip.split('/').collect();
            if split[0].contains(':') {
                // It's IPv6
                let ip_network: Ipv6Addr = split[0]
                    .parse()
                    .map_err(|e| IpRangeError::IpParseError { e: Box::new(e) })?;
                let ip = IpNetwork::new(
                    ip_network,
                    split[1].parse().map_err(|_| IpRangeError::InvalidNetmask)?,
                )
                .map_err(|e| IpRangeError::InvalidNetwork { e: Box::new(e) })?;
                allowed.insert(ip, true);
            } else {
                // It's IPv4
                let ip_network: Ipv4Addr = split[0]
                    .parse()
                    .map_err(|e| IpRangeError::IpParseError { e: Box::new(e) })?;
                let ip = IpNetwork::new(
                    ip_network,
                    split[1].parse().map_err(|_| IpRangeError::InvalidNetmask)?,
                )
                .map_err(|e| IpRangeError::InvalidNetwork { e: Box::new(e) })?;
                allowed.insert(ip, true);
            }
        }
        Ok(allowed)
    }
}

#[derive(thiserror::Error, Debug)]
pub enum IpRangeError {
    #[error("Unable to parse IP range: {e:?}")]
    IpParseError { e: Box<dyn std::error::Error> },
    #[error("Invalid network: {e:?}")]
    InvalidNetwork { e: Box<dyn std::error::Error> },
    #[error("Invalid netmask")]
    InvalidNetmask,
}
