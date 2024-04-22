use crate::errors::UispIntegrationError;
use ip_network::IpNetwork;
use ip_network_table::IpNetworkTable;
use lqos_config::Config;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use tracing::info;

pub struct IpRanges {
    allowed: IpNetworkTable<bool>,
    ignored: IpNetworkTable<bool>,
}

impl IpRanges {
    pub fn new(config: &Config) -> Result<Self, UispIntegrationError> {
        info!("Building allowed/excluded IP range lookups from configuration file");

        let mut allowed = IpNetworkTable::new();
        let mut ignored = IpNetworkTable::new();

        for allowed_ip in config.ip_ranges.allow_subnets.iter() {
            let split: Vec<_> = allowed_ip.split('/').collect();
            if (split[0].contains(':')) {
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
            if (split[0].contains(':')) {
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

    pub fn is_permitted(&self, ip: IpAddr, subnet: u8) -> bool {
        if let Some(allow) = self.allowed.longest_match(ip) {
            if let Some(deny) = self.ignored.longest_match(ip) {
                return false;
            }
            return true;
        }
        false
    }
}
