//! Queue Generation definitions (originally from ispConfig.py)

use allocative::Allocative;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Queue application mode.
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq, Default, Allocative)]
#[serde(rename_all = "snake_case")]
pub enum QueueMode {
    /// Apply LibreQoS queueing and shaping.
    #[default]
    Shape,
    /// Observe only; do not apply managed TC objects.
    Observe,
}

impl QueueMode {
    /// Returns `true` when the queue mode is observe-only.
    pub const fn is_observe(self) -> bool {
        matches!(self, Self::Observe)
    }
}

#[derive(Clone, Debug, PartialEq, Allocative)]
pub struct QueueConfig {
    /// Which SQM to use by default
    pub default_sqm: String,

    /// Queue application mode.
    pub queue_mode: QueueMode,

    /// Upstream bandwidth total - upload (Mbps; a decimal is accepted).
    pub uplink_bandwidth_mbps: f64,

    /// Downstream bandwidth total - download (Mbps; a decimal is accepted).
    pub downlink_bandwidth_mbps: f64,

    /// Downstream bandwidth per interface queue
    pub generated_pn_download_mbps: u64,

    /// Upstream bandwidth per interface queue
    pub generated_pn_upload_mbps: u64,

    /// Should shell commands actually execute, or just be printed?
    pub dry_run: bool,

    /// Should `sudo` be prefixed on commands?
    pub sudo: bool,

    /// Should we override the number of available queues?
    pub override_available_queues: Option<u32>,

    /// DPDK scheduler WORKER count (`--workers`); `None` = derive via the
    /// installer's ceiling-table rule (docs/zero_manual_ports_installer_card.md
    /// B.4). NOT the same axis as `override_available_queues` above: that one
    /// tunes the CAKE/placement QUEUE BUILD (the lqos_python/lqos_topology
    /// subsystem); this one sizes the DPDK worker pool. Two unrelated knobs.
    pub workers: Option<u32>,

    /// Should we invoke the binpacking algorithm to optimize flat
    /// networks?
    pub use_binpacking: bool,
}

impl Serialize for QueueConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("QueueConfig", 12)?;
        state.serialize_field("default_sqm", &self.default_sqm)?;
        state.serialize_field("queue_mode", &self.queue_mode)?;
        // Preserve the legacy field during rewrites so older binaries that still
        // require it continue to parse upgraded configs successfully.
        state.serialize_field("monitor_only", &self.queue_mode.is_observe())?;
        state.serialize_field("uplink_bandwidth_mbps", &self.uplink_bandwidth_mbps)?;
        state.serialize_field("downlink_bandwidth_mbps", &self.downlink_bandwidth_mbps)?;
        state.serialize_field(
            "generated_pn_download_mbps",
            &self.generated_pn_download_mbps,
        )?;
        state.serialize_field("generated_pn_upload_mbps", &self.generated_pn_upload_mbps)?;
        state.serialize_field("dry_run", &self.dry_run)?;
        state.serialize_field("sudo", &self.sudo)?;
        state.serialize_field("override_available_queues", &self.override_available_queues)?;
        state.serialize_field("workers", &self.workers)?;
        state.serialize_field("use_binpacking", &self.use_binpacking)?;
        state.end()
    }
}

#[derive(Deserialize)]
#[serde(default)]
struct QueueConfigCompat {
    default_sqm: String,
    queue_mode: Option<QueueMode>,
    monitor_only: bool,
    uplink_bandwidth_mbps: f64,
    downlink_bandwidth_mbps: f64,
    generated_pn_download_mbps: u64,
    generated_pn_upload_mbps: u64,
    dry_run: bool,
    sudo: bool,
    override_available_queues: Option<u32>,
    workers: Option<u32>,
    use_binpacking: bool,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            default_sqm: "cake diffserv4".to_string(),
            queue_mode: QueueMode::Shape,
            uplink_bandwidth_mbps: 1_000.0,
            downlink_bandwidth_mbps: 1_000.0,
            generated_pn_download_mbps: 1_000,
            generated_pn_upload_mbps: 1_000,
            dry_run: false,
            sudo: false,
            override_available_queues: None,
            workers: None,
            use_binpacking: false,
        }
    }
}

impl QueueConfig {
    /// Sets the queue application mode.
    pub fn set_queue_mode(&mut self, queue_mode: QueueMode) {
        self.queue_mode = queue_mode;
    }
}

impl Default for QueueConfigCompat {
    fn default() -> Self {
        let defaults = QueueConfig::default();
        Self {
            default_sqm: defaults.default_sqm.clone(),
            queue_mode: None,
            monitor_only: false,
            uplink_bandwidth_mbps: defaults.uplink_bandwidth_mbps,
            downlink_bandwidth_mbps: defaults.downlink_bandwidth_mbps,
            generated_pn_download_mbps: defaults.generated_pn_download_mbps,
            generated_pn_upload_mbps: defaults.generated_pn_upload_mbps,
            dry_run: defaults.dry_run,
            sudo: defaults.sudo,
            override_available_queues: defaults.override_available_queues,
            workers: defaults.workers,
            use_binpacking: defaults.use_binpacking,
        }
    }
}

impl<'de> Deserialize<'de> for QueueConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let compat = QueueConfigCompat::deserialize(deserializer)?;
        let queue_mode = compat.queue_mode.unwrap_or({
            if compat.monitor_only {
                QueueMode::Observe
            } else {
                QueueMode::Shape
            }
        });

        let mut cfg = Self {
            default_sqm: compat.default_sqm,
            queue_mode,
            uplink_bandwidth_mbps: compat.uplink_bandwidth_mbps,
            downlink_bandwidth_mbps: compat.downlink_bandwidth_mbps,
            generated_pn_download_mbps: compat.generated_pn_download_mbps,
            generated_pn_upload_mbps: compat.generated_pn_upload_mbps,
            dry_run: compat.dry_run,
            sudo: compat.sudo,
            override_available_queues: compat.override_available_queues,
            workers: compat.workers,
            use_binpacking: compat.use_binpacking,
        };
        cfg.set_queue_mode(queue_mode);
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::{QueueConfig, QueueMode};

    #[test]
    fn deserialize_legacy_monitor_only_maps_to_observe() {
        let parsed: QueueConfig =
            toml::from_str("default_sqm = \"cake diffserv4\"\nmonitor_only = true\n")
                .expect("legacy monitor_only config should deserialize");
        assert_eq!(parsed.queue_mode, QueueMode::Observe);
    }

    #[test]
    fn deserialize_without_queue_mode_defaults_to_shape() {
        let parsed: QueueConfig = toml::from_str("default_sqm = \"cake diffserv4\"\n")
            .expect("queue config without queue_mode should deserialize");
        assert_eq!(parsed.queue_mode, QueueMode::Shape);
    }

    #[test]
    fn deserialize_prefers_queue_mode_over_legacy_monitor_only() {
        let parsed: QueueConfig = toml::from_str(
            "default_sqm = \"cake diffserv4\"\nqueue_mode = \"shape\"\nmonitor_only = true\n",
        )
        .expect("mixed config should deserialize");
        assert_eq!(parsed.queue_mode, QueueMode::Shape);
    }

    #[test]
    fn serialize_queue_config_preserves_legacy_monitor_only_field() {
        let mut config = QueueConfig::default();
        config.set_queue_mode(QueueMode::Observe);
        let serialized = toml::to_string(&config).expect("queue config should serialize");
        assert!(serialized.contains("queue_mode = \"observe\""));
        assert!(
            serialized.contains("monitor_only = true"),
            "serialized config should preserve legacy monitor_only for compatibility"
        );
    }

    #[test]
    fn serialize_shape_queue_config_sets_legacy_monitor_only_false() {
        let mut config = QueueConfig::default();
        config.set_queue_mode(QueueMode::Shape);
        let serialized = toml::to_string(&config).expect("queue config should serialize");
        assert!(serialized.contains("queue_mode = \"shape\""));
        assert!(
            serialized.contains("monitor_only = false"),
            "serialized config should preserve legacy monitor_only=false for compatibility"
        );
    }
}
