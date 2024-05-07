//! Interface tuning instructions

use serde::{Deserialize, Serialize};

/// Represents a set of `sysctl` and `ethtool` tweaks that may be
/// applied (in place of the previous version's offload service)
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Tunables {
    /// Should the `irq_balance` system service be stopped?
    pub stop_irq_balance: bool,

    /// Set the netdev budget (usecs)
    pub netdev_budget_usecs: u32,

    /// Set the netdev budget (packets)
    pub netdev_budget_packets: u32,

    /// Set the RX side polling frequency
    pub rx_usecs: u32,

    /// Set the TX side polling frequency
    pub tx_usecs: u32,

    /// Disable RXVLAN offloading? You generally want to do this.
    pub disable_rxvlan: bool,

    /// Disable TXVLAN offloading? You generally want to do this.
    pub disable_txvlan: bool,

    /// A list of `ethtool` offloads to be disabled.
    /// The default list is: [ "gso", "tso", "lro", "sg", "gro" ]
    pub disable_offload: Vec<String>,
}

impl Default for Tunables {
    fn default() -> Self {
        Self {
            stop_irq_balance: true,
            netdev_budget_usecs: 8000,
            netdev_budget_packets: 300,
            rx_usecs: 8,
            tx_usecs: 8,
            disable_rxvlan: true,
            disable_txvlan: true,
            disable_offload: vec![
                "gso".to_string(),
                "tso".to_string(),
                "lro".to_string(),
                "sg".to_string(),
                "gro".to_string(),
            ],
        }
    }
}
