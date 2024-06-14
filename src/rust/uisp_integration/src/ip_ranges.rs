use crate::errors::UispIntegrationError;
use ip_network::IpNetwork;
use ip_network_table::IpNetworkTable;
use lqos_config::Config;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use tracing::info;

/// Represents a set of IP ranges that are allowed or ignored.
pub struct IpRanges {
    /// The allowed IP ranges
    allowed: IpNetworkTable<bool>,
    /// The ignored IP ranges
    ignored: IpNetworkTable<bool>,
}

impl IpRanges {
    /// Creates a new IpRanges from a configuration.
    pub fn new(config: &Config) -> Result<Self, UispIntegrationError> {
        info!("Building allowed/excluded IP range lookups from configuration file");

        let mut allowed = IpNetworkTable::new();
        let mut ignored = IpNetworkTable::new();

        for allowed_ip in config.ip_ranges.allow_subnets.iter() {
            let split: Vec<_> = allowed_ip.split('/').collect();
            if split[0].contains(':') {
                // It's IPv6
                let ip_network: Ipv6Addr = split[0].parse().unwrap();
                let ip = IpNetwork::new(ip_network, split[1].parse().unwrap()).unwrap();
                allowed.insert(ip, true);
            } else {
                // It's IPv4
                let ip_network: Ipv4Addr = split[0].parse().unwrap();
                let ip = IpNetwork::new(ip_network, split[1].parse().unwrap()).unwrap();
                allowed.insert(ip, true);
            }
        }
        for excluded_ip in config.ip_ranges.ignore_subnets.iter() {
            let split: Vec<_> = excluded_ip.split('/').collect();
            if split[0].contains(':') {
                // It's IPv6
                let ip_network: Ipv6Addr = split[0].parse().unwrap();
                let ip = IpNetwork::new(ip_network, split[1].parse().unwrap()).unwrap();
                ignored.insert(ip, true);
            } else {
                // It's IPv4
                let ip_network: Ipv4Addr = split[0].parse().unwrap();
                let ip = IpNetwork::new(ip_network, split[1].parse().unwrap()).unwrap();
                ignored.insert(ip, true);
            }
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
        //println!("Checking: {:?}", ip);
        if let Some(_allow) = self.allowed.longest_match(ip) {
            if let Some(_deny) = self.ignored.longest_match(ip) {
                return false;
            }
            return true;
        }
        false
    }
}
