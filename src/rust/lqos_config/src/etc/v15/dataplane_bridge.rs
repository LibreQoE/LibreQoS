//! DPDK bridge dataplane configuration (the userspace `libreqos-bridge`
//! process, not the kernel bridge interfaces at `bridge`).
//!
//! E2 stats + health (docs card `lqosd_broker_emission_card.md` §5): when
//! this optional section is present (or `LIBREQOS_ANALYTICS_SOCKET` is set
//! in the environment), lqosd polls the bridge's analytics socket for
//! per-circuit counters and the supervision `HealthReport`. The section is
//! OPT-IN so kernel-dataplane installations are untouched.

use allocative::Allocative;
use serde::{Deserialize, Serialize};

/// Default analytics socket path. Must match the bridge-side
/// `LIBREQOS_ANALYTICS_SOCKET` deployment value; the environment variable
/// wins over this field when set on both sides of a pair.
pub const DEFAULT_ANALYTICS_SOCKET: &str = "/dev/shm/libreqos-analytics.sock";

fn default_analytics_socket() -> String {
    DEFAULT_ANALYTICS_SOCKET.to_string()
}

fn default_stats_poll_interval_ms() -> u64 {
    1_000
}

fn default_health_probe_interval_secs() -> u64 {
    10
}

fn default_health_fails_to_down() -> u32 {
    3
}

/// DPDK bridge dataplane settings (all fields defaulted; the section's
/// PRESENCE is the activation switch for the E2 actors).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Allocative)]
pub struct DataplaneBridgeConfig {
    /// Unix socket path of the bridge analytics endpoint. The
    /// `LIBREQOS_ANALYTICS_SOCKET` environment variable (when set)
    /// overrides this value.
    #[serde(default = "default_analytics_socket")]
    pub analytics_socket: String,

    /// Per-circuit stats polling interval in milliseconds.
    ///
    /// Clamped to a 250 ms floor at use; the counters are cumulative so a
    /// shorter interval only changes rate resolution.
    #[serde(default = "default_stats_poll_interval_ms")]
    pub stats_poll_interval_ms: u64,

    /// Supervisor health probe cadence in seconds (supervision contract
    /// §8: 10 s default, product-configurable).
    #[serde(default = "default_health_probe_interval_secs")]
    pub health_probe_interval_secs: u64,

    /// Consecutive failed probes before the Supervisor verdict goes Down
    /// (supervision contract §8: default 3 — "not trigger-happy, not
    /// come-back-after-lunch"). A Busy reply or a single timeout never
    /// escalates (MEDIUM-1).
    #[serde(default = "default_health_fails_to_down")]
    pub health_fails_to_down: u32,
}

impl Default for DataplaneBridgeConfig {
    fn default() -> Self {
        Self {
            analytics_socket: default_analytics_socket(),
            stats_poll_interval_ms: default_stats_poll_interval_ms(),
            health_probe_interval_secs: default_health_probe_interval_secs(),
            health_fails_to_down: default_health_fails_to_down(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DataplaneBridgeConfig;

    #[test]
    fn empty_section_takes_pinned_defaults() {
        let parsed: DataplaneBridgeConfig = toml::from_str("").expect("empty section parses");
        assert_eq!(parsed, DataplaneBridgeConfig::default());
        assert_eq!(parsed.analytics_socket, "/dev/shm/libreqos-analytics.sock");
        assert_eq!(parsed.stats_poll_interval_ms, 1_000);
        assert_eq!(parsed.health_probe_interval_secs, 10);
        assert_eq!(parsed.health_fails_to_down, 3);
    }

    #[test]
    fn overrides_apply() {
        let parsed: DataplaneBridgeConfig = toml::from_str(
            "analytics_socket = \"/run/analytics.sock\"\nstats_poll_interval_ms = 500\nhealth_probe_interval_secs = 5\nhealth_fails_to_down = 2\n",
        )
        .expect("overrides parse");
        assert_eq!(parsed.analytics_socket, "/run/analytics.sock");
        assert_eq!(parsed.stats_poll_interval_ms, 500);
        assert_eq!(parsed.health_probe_interval_secs, 5);
        assert_eq!(parsed.health_fails_to_down, 2);
    }
}
