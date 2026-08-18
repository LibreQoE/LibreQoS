//! Shared mapped-circuit licensing definitions.

use std::collections::HashSet;
use std::net::IpAddr;

/// Maximum number of valid mapped circuits allowed without an entitlement.
pub const DEFAULT_MAPPED_CIRCUIT_LIMIT: u64 = 1_000;

/// Returns whether an IPv4 prefix length is valid.
pub const fn is_valid_ipv4_prefix(prefix: u32) -> bool {
    prefix <= 32
}

/// Returns whether an IPv6 prefix length is valid.
pub const fn is_valid_ipv6_prefix(prefix: u32) -> bool {
    prefix <= 128
}

/// Returns whether an IP or CIDR string is a valid IPv4 or IPv6 mapping.
pub fn is_valid_ip_mapping_text(mapping: &str) -> bool {
    let mapping = mapping.trim();
    if mapping.is_empty() {
        return false;
    }

    let (address, prefix) = match mapping.split_once('/') {
        Some((address, prefix)) => {
            let Ok(prefix) = prefix.parse::<u32>() else {
                return false;
            };
            (address, Some(prefix))
        }
        None => (mapping, None),
    };

    match address.parse::<IpAddr>() {
        Ok(IpAddr::V4(_)) => prefix.is_none_or(is_valid_ipv4_prefix),
        Ok(IpAddr::V6(_)) => prefix.is_none_or(is_valid_ipv6_prefix),
        Err(_) => false,
    }
}

/// Returns unique circuit hashes that have at least one valid IP mapping.
///
/// Duplicate hashes count once, in first-seen order.
pub fn unique_mapped_circuit_hashes(circuit_hashes: impl IntoIterator<Item = i64>) -> Vec<i64> {
    let mut seen = HashSet::new();
    circuit_hashes
        .into_iter()
        .filter(|circuit_hash| seen.insert(*circuit_hash))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{is_valid_ip_mapping_text, unique_mapped_circuit_hashes};

    #[test]
    fn mapping_text_requires_valid_address_and_prefix() {
        assert!(is_valid_ip_mapping_text("192.0.2.1/32"));
        assert!(is_valid_ip_mapping_text("2001:db8::/64"));
        assert!(!is_valid_ip_mapping_text("192.0.2.1/64"));
        assert!(!is_valid_ip_mapping_text("2001:db8::/129"));
        assert!(!is_valid_ip_mapping_text("not-an-address"));
    }

    #[test]
    fn unique_mapped_hashes_deduplicate() {
        assert_eq!(unique_mapped_circuit_hashes([10, 10, 30]), vec![10, 30]);
    }
}
