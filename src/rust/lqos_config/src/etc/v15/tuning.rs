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

    /// DEPRECATED one-release alias for [`Self::worker_capacity_mbps`]
    /// (`lqos-74t.191`). It used to mean a **pre-headroom** budget; it is now
    /// interpreted as the same EFFECTIVE per-worker capacity in Mbps (no
    /// headroom). The bridge logs a deprecation warning when it uses this key.
    /// Prefer `worker_capacity_mbps`; this key is not written to the template
    /// and will be retired after one release.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement_budget_mbps: Option<u64>,

    /// Per-worker EXPECTED-THROUGHPUT capacity in Mbps — the EFFECTIVE soft
    /// placement packing objective, not a pre-headroom value
    /// (`lqos-74t.191`; default [`WORKER_CAPACITY_MBPS_DEFAULT`] = 1,125; a
    /// high-performance preset uses 2,000). The hard fairness guard remains the
    /// derived circuit-COUNT cap; this value is NOT a commit/ceiling cap and
    /// never makes a placement infeasible. Absent = the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_capacity_mbps: Option<u64>,
}

/// Default per-worker EFFECTIVE capacity (Mbps). Not a cap.
pub const WORKER_CAPACITY_MBPS_DEFAULT: u64 = 1_125;

impl Tunables {
    /// The effective per-worker capacity in Mbps: `worker_capacity_mbps`, or
    /// the deprecated `placement_budget_mbps` alias, or the 1,125 default.
    /// Never consult this to accept/reject a placement or a move — the hard
    /// fairness guard is the derived circuit-COUNT cap.
    pub fn worker_capacity_mbps(&self) -> u64 {
        self.worker_capacity_mbps
            .or(self.placement_budget_mbps)
            .unwrap_or(WORKER_CAPACITY_MBPS_DEFAULT)
    }
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
            worker_capacity_mbps: None,
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

    #[test]
    fn worker_capacity_defaults_to_1125_and_has_a_2000_preset() {
        let rendered = full_default_toml();
        assert!(
            !rendered.contains("worker_capacity_mbps"),
            "the struct default keeps the optional key absent"
        );
        let parsed: Tunables = toml::from_str(&rendered).expect("parses");
        assert_eq!(
            parsed.worker_capacity_mbps(),
            super::WORKER_CAPACITY_MBPS_DEFAULT
        );
        assert_eq!(parsed.worker_capacity_mbps(), 1_125);

        let raw = format!("{}\nworker_capacity_mbps = 2000\n", rendered);
        let parsed: Tunables = toml::from_str(&raw).expect("parses");
        assert_eq!(parsed.worker_capacity_mbps(), 2_000);
    }

    #[test]
    fn emitted_template_exposes_the_1125_default() {
        let raw = include_str!("example.toml");
        assert!(
            raw.contains("worker_capacity_mbps = 1125"),
            "example.toml must expose the effective worker-capacity knob"
        );
        let document: toml::Value = toml::from_str(raw).expect("example toml parses");
        let tuning = document
            .get("tuning")
            .expect("example.toml has a [tuning] table")
            .clone();
        let parsed: Tunables = tuning.try_into().expect("example tuning parses");
        assert_eq!(parsed.worker_capacity_mbps(), 1_125);
    }

    #[test]
    fn deprecated_placement_budget_is_an_alias_and_worker_capacity_wins() {
        let rendered = full_default_toml();

        // One-release migration: an old key still resolves as the SAME
        // effective Mbps number (no pre-headroom conversion).
        let legacy = format!("{}\nplacement_budget_mbps = 1500\n", rendered);
        let parsed: Tunables = toml::from_str(&legacy).expect("parses");
        assert_eq!(parsed.worker_capacity_mbps(), 1_500);

        // If both are present, the canonical key wins.
        let both = format!(
            "{}\nplacement_budget_mbps = 2000\nworker_capacity_mbps = 1125\n",
            rendered
        );
        let parsed: Tunables = toml::from_str(&both).expect("parses");
        assert_eq!(parsed.worker_capacity_mbps(), 1_125);
    }
}
