use crate::errors::UispIntegrationError;
use ip_network::IpNetwork;
use ip_network_table::IpNetworkTable;
use lqos_config::Config;
use std::net::IpAddr;
use tracing::{info, warn};

/// Represents a set of IP ranges that are allowed or ignored.
pub struct IpRanges {
    /// The allowed IP ranges
    allowed: IpNetworkTable<bool>,
    /// The ignored IP ranges
    ignored: IpNetworkTable<bool>,
}

impl IpRanges {
    fn parse_configured_subnet(raw: &str) -> Result<IpNetwork, UispIntegrationError> {
        let Some((addr, prefix)) = raw.split_once('/') else {
            warn!("Ignoring invalid UISP subnet '{raw}': missing prefix length");
            return Err(UispIntegrationError::BadIpRange(raw.to_string()));
        };
        let prefix = prefix.parse::<u8>().map_err(|err| {
            warn!("Ignoring invalid UISP subnet '{raw}': invalid prefix length ({err})");
            UispIntegrationError::BadIpRange(raw.to_string())
        })?;
        let ip = addr.parse::<IpAddr>().map_err(|err| {
            warn!("Ignoring invalid UISP subnet '{raw}': invalid IP address ({err})");
            UispIntegrationError::BadIpRange(raw.to_string())
        })?;

        match ip {
            IpAddr::V4(ip) => IpNetwork::new(ip, prefix),
            IpAddr::V6(ip) => IpNetwork::new(ip, prefix),
        }
        .map_err(|err| {
            warn!("Ignoring invalid UISP subnet '{raw}': invalid network ({err})");
            UispIntegrationError::BadIpRange(raw.to_string())
        })
    }

    /// Creates a new IpRanges from a configuration.
    pub fn new(config: &Config) -> Result<Self, UispIntegrationError> {
        info!("Building allowed/excluded IP range lookups from configuration file");

        let mut allowed = IpNetworkTable::new();
        let mut ignored = IpNetworkTable::new();

        for allowed_ip in config.ip_ranges.allow_subnets.iter() {
            let ip = Self::parse_configured_subnet(allowed_ip)?;
            allowed.insert(ip, true);
        }
        for excluded_ip in config.ip_ranges.ignore_subnets.iter() {
            let ip = Self::parse_configured_subnet(excluded_ip)?;
            ignored.insert(ip, true);
        }
        info!(
            "{} allowed IP ranges, {} ignored IP ranges",
            allowed.len().0,
            ignored.len().0
        );

        Ok(Self { allowed, ignored })
    }

    /// Checks if an IP address is permitted.
    pub fn is_permitted(&self, ip: IpAddr) -> bool {
        if let Some(_allow) = self.allowed.longest_match(ip) {
            if let Some(_deny) = self.ignored.longest_match(ip) {
                return false;
            }
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_subnets_reject_malformed_values_without_panicking() {
        assert!(IpRanges::parse_configured_subnet("not-a-cidr").is_err());
        assert!(IpRanges::parse_configured_subnet("192.0.2.0/nope").is_err());
        assert!(IpRanges::parse_configured_subnet("192.0.2.1/33").is_err());
        assert!(IpRanges::parse_configured_subnet("2001:db8::/129").is_err());
    }
}
