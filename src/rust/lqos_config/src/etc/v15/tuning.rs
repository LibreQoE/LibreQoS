//! Interface tuning instructions

use allocative::Allocative;
use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

/// Represents a set of `sysctl` and `ethtool` tweaks that may be
/// applied (in place of the previous version's offload service)
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Allocative)]
pub struct Tunables {
    /// Should LibreQoS set the CPU governor to `performance` during tuning?
    #[serde(default = "default_true")]
    pub set_cpu_governor_performance: bool,

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

    /// Optional per-worker DPDK placement budget in Mbps, **pre-headroom**
    /// (2000 = 2.0G; effective 1.5G at the default 25% headroom). Absent =
    /// the compiled 2.0G default. `[tuning]` is tolerant, so an older lqosd
    /// ignores this key instead of failing the `[dataplane]` strict parse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement_budget_mbps: Option<u64>,
}

impl Default for Tunables {
    fn default() -> Self {
        Self {
            set_cpu_governor_performance: true,
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
            placement_budget_mbps: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Tunables;

    fn full_default_toml() -> String {
        toml::to_string(&Tunables::default()).expect("serializes")
    }

    #[test]
    fn placement_budget_is_optional_and_absent_by_default() {
        let rendered = full_default_toml();
        assert!(
            !rendered.contains("placement_budget_mbps"),
            "the default template must not emit the optional key"
        );
        let parsed: Tunables = toml::from_str(&rendered).expect("parses");
        assert_eq!(parsed.placement_budget_mbps, None);
    }

    #[test]
    fn placement_budget_parses_integer_mbps() {
        let raw = format!("{}\nplacement_budget_mbps = 2000\n", full_default_toml());
        let parsed: Tunables = toml::from_str(&raw).expect("parses");
        assert_eq!(parsed.placement_budget_mbps, Some(2000));
    }

    #[test]
    fn placement_budget_rejects_negative_or_non_integer_values() {
        for bad in ["-1", "12.5", "nope"] {
            let raw = format!("{}\nplacement_budget_mbps = {bad}\n", full_default_toml());
            assert!(
                toml::from_str::<Tunables>(&raw).is_err(),
                "{bad} must not parse as Option<u64> Mbps"
            );
        }
    }
}
