use nix::ifaddrs::{getifaddrs, InterfaceAddress};

/// Returns a vector of (id, name) tuples for all network interfaces.
pub fn get_nic_list() -> Vec<InterfaceAddress> {
    getifaddrs()
        .unwrap()
        .collect()
}