//! Queue Generation definitions (originally from ispConfig.py)

use allocative::Allocative;
use serde::{Deserialize, Deserializer, Serialize};

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

#[derive(Clone, Serialize, Debug, PartialEq, Allocative)]
pub struct QueueConfig {
    /// Which SQM to use by default
    pub default_sqm: String,

    /// Queue application mode.
    pub queue_mode: QueueMode,

    /// Upstream bandwidth total - download
    pub uplink_bandwidth_mbps: u64,

    /// Downstream bandwidth total - upload
    pub downlink_bandwidth_mbps: u64,

    /// Upstream bandwidth per interface queue
    pub generated_pn_download_mbps: u64,

    /// Downstream bandwidth per interface queue
    pub generated_pn_upload_mbps: u64,

    /// Should shell commands actually execute, or just be printed?
    pub dry_run: bool,

    /// Should `sudo` be prefixed on commands?
    pub sudo: bool,

    /// Should we override the number of available queues?
    pub override_available_queues: Option<u32>,

    /// Should we invoke the binpacking algorithm to optimize flat
    /// networks?
    pub use_binpacking: bool,

    /// Enable lazy queue creation (only create circuit queues when traffic is detected)
    pub lazy_queues: Option<LazyQueueMode>,

    /// Expiration time in seconds for unused lazy queues (None = never expire)
    pub lazy_expire_seconds: Option<u64>,

    /// Hold-off on creating lazy queues until this many bytes have been seen
    pub lazy_threshold_bytes: Option<u64>,

    /// Auto-change queues to fq_codel if they are greater than or equal to X Mbps. Defaults to 1000.
    pub fast_queues_fq_codel: Option<f64>,
}

#[derive(Deserialize)]
#[serde(default)]
struct QueueConfigCompat {
    default_sqm: String,
    queue_mode: Option<QueueMode>,
    monitor_only: bool,
    uplink_bandwidth_mbps: u64,
    downlink_bandwidth_mbps: u64,
    generated_pn_download_mbps: u64,
    generated_pn_upload_mbps: u64,
    dry_run: bool,
    sudo: bool,
    override_available_queues: Option<u32>,
    use_binpacking: bool,
    lazy_queues: Option<LazyQueueMode>,
    lazy_expire_seconds: Option<u64>,
    lazy_threshold_bytes: Option<u64>,
    fast_queues_fq_codel: Option<f64>,
}

/// Lazy queue creation modes
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Default, Allocative)]
pub enum LazyQueueMode {
    /// No lazy queue creation
    #[default]
    No,
    /// HTB queues for circuits are created on build, but CAKE classes are created on demand
    Htb,
    /// Full lazy queue creation, both HTB queues and CAKE classes are created on demand.
    Full,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            default_sqm: "cake diffserv4".to_string(),
            queue_mode: QueueMode::Shape,
            uplink_bandwidth_mbps: 1_000,
            downlink_bandwidth_mbps: 1_000,
            generated_pn_download_mbps: 1_000,
            generated_pn_upload_mbps: 1_000,
            dry_run: false,
            sudo: false,
            override_available_queues: None,
            use_binpacking: false,
            lazy_queues: None, // Default to disabled for backward compatibility
            lazy_expire_seconds: Some(600), // 10 minutes default
            lazy_threshold_bytes: None,
            fast_queues_fq_codel: None,
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
            use_binpacking: defaults.use_binpacking,
            lazy_queues: defaults.lazy_queues.clone(),
            lazy_expire_seconds: defaults.lazy_expire_seconds,
            lazy_threshold_bytes: defaults.lazy_threshold_bytes,
            fast_queues_fq_codel: defaults.fast_queues_fq_codel,
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
            use_binpacking: compat.use_binpacking,
            lazy_queues: compat.lazy_queues,
            lazy_expire_seconds: compat.lazy_expire_seconds,
            lazy_threshold_bytes: compat.lazy_threshold_bytes,
            fast_queues_fq_codel: compat.fast_queues_fq_codel,
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
    fn serialize_queue_config_omits_legacy_monitor_only_field() {
        let mut config = QueueConfig::default();
        config.set_queue_mode(QueueMode::Observe);
        let serialized = toml::to_string(&config).expect("queue config should serialize");
        assert!(serialized.contains("queue_mode = \"observe\""));
        assert!(
            !serialized.contains("monitor_only"),
            "serialized config should not re-emit deprecated monitor_only"
        );
    }
}
