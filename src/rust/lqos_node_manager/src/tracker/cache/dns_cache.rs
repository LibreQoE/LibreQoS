//! Implements a lock-free DNS least-recently-used DNS cache.

use std::net::IpAddr;
use dashmap::DashMap;
use dns_lookup::lookup_addr;
use lqos_utils::unix_time::unix_now;
use once_cell::sync::Lazy;

const CACHE_SIZE: usize = 1000;

struct DnsEntry {
    hostname: String,
    last_accessed: u64,
}

static DNS_CACHE: Lazy<DashMap<IpAddr, DnsEntry>> = Lazy::new(|| DashMap::with_capacity(CACHE_SIZE));

pub fn lookup_dns(ip: IpAddr) -> String {
    // If the cached value exists, just return it
    if let Some(mut dns) = DNS_CACHE.get_mut(&ip) {
        if let Ok(now) = unix_now() {
            dns.last_accessed = now;
        }
        return dns.hostname.clone();
    }

    // If it doesn't, we'll be adding it.
    if DNS_CACHE.len() >= CACHE_SIZE {
        let mut entries : Vec<(IpAddr, u64)> = DNS_CACHE.iter().map(|v| (*v.key(), v.last_accessed)).collect();
        entries.sort_by(|a,b| b.1.cmp(&a.1));
        DNS_CACHE.remove(&entries[0].0);
    }
    let hostname = lookup_addr(&ip).unwrap_or(ip.to_string());
    DNS_CACHE.insert(ip, DnsEntry { hostname, last_accessed: unix_now().unwrap_or(0) });


    String::new()
}