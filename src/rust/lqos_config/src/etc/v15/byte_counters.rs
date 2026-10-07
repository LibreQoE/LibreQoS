//! Configuration for optional cumulative byte counters.
//!
//! When enabled, `lqosd` accumulates per-circuit byte totals from the
//! one-second throughput tracker and exposes cumulative per-circuit and
//! per-node counters on the local bus.

use allocative::Allocative;
use serde::{Deserialize, Serialize};

/// Configuration for optional cumulative byte counters.
#[derive(Clone, Default, Serialize, Deserialize, Debug, PartialEq, Allocative)]
pub struct ByteCountersConfig {
    /// Enables cumulative per-circuit and per-node byte counters.
    ///
    /// Defaults to `false` when the `[byte_counters]` section is absent so
    /// existing configuration files keep their current behavior.
    #[serde(default)]
    pub enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::ByteCountersConfig;

    #[test]
    fn empty_section_defaults_to_disabled() {
        let config: ByteCountersConfig =
            toml::from_str("").expect("empty byte_counters section should parse");
        assert!(!config.enabled);
    }

    #[test]
    fn section_parses_enabled_flag() {
        let config: ByteCountersConfig =
            toml::from_str("enabled = true").expect("enabled flag should parse");
        assert!(config.enabled);
    }
}
