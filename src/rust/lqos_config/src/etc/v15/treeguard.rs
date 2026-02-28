//! TreeGuard (intelligent node management) configuration.

use allocative::Allocative;
use serde::{Deserialize, Serialize};

/// TreeGuard (intelligent node management) configuration.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Allocative)]
#[serde(default)]
pub struct TreeguardConfig {
    /// Whether TreeGuard is enabled.
    pub enabled: bool,
    /// Whether TreeGuard operates in dry-run mode (no persistent writes or live applies).
    pub dry_run: bool,
    /// TreeGuard tick cadence in seconds.
    pub tick_seconds: u64,
    /// CPU-related behavior configuration.
    pub cpu: TreeguardCpuConfig,
    /// Link/node virtualization configuration.
    pub links: TreeguardLinksConfig,
    /// Circuit SQM switching configuration.
    pub circuits: TreeguardCircuitsConfig,
    /// QoO guardrail configuration.
    pub qoo: TreeguardQooConfig,
}

impl Default for TreeguardConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            dry_run: true,
            tick_seconds: 1,
            cpu: TreeguardCpuConfig::default(),
            links: TreeguardLinksConfig::default(),
            circuits: TreeguardCircuitsConfig::default(),
            qoo: TreeguardQooConfig::default(),
        }
    }
}

/// TreeGuard CPU control mode.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Allocative)]
#[serde(rename_all = "snake_case")]
pub enum TreeguardCpuMode {
    /// TreeGuard makes CPU-saving decisions based on CPU usage and other guardrails.
    CpuAware,
    /// TreeGuard ignores CPU usage and uses only traffic/RTT/QoO guardrails.
    TrafficRttOnly,
}

impl Default for TreeguardCpuMode {
    fn default() -> Self {
        Self::CpuAware
    }
}

/// TreeGuard CPU-related configuration.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Allocative)]
#[serde(default)]
pub struct TreeguardCpuConfig {
    /// CPU control mode.
    pub mode: TreeguardCpuMode,
    /// CPU usage percentage at/above which TreeGuard may take CPU-saving actions.
    pub cpu_high_pct: u8,
    /// CPU usage percentage at/below which TreeGuard should revert CPU-saving actions.
    pub cpu_low_pct: u8,
}

impl Default for TreeguardCpuConfig {
    fn default() -> Self {
        Self {
            mode: TreeguardCpuMode::CpuAware,
            cpu_high_pct: 75,
            cpu_low_pct: 55,
        }
    }
}

/// TreeGuard link/node virtualization configuration.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Allocative)]
#[serde(default)]
pub struct TreeguardLinksConfig {
    /// Whether link/node virtualization is enabled.
    pub enabled: bool,
    /// Whether TreeGuard may manage all non-root nodes in `network.json`.
    ///
    /// When enabled, the `nodes` allowlist is ignored.
    #[serde(default)]
    pub all_nodes: bool,
    /// Node allowlist: network.json node names that TreeGuard may manage.
    pub nodes: Vec<String>,
    /// Utilization percentage below which a link is considered idle.
    pub idle_util_pct: f32,
    /// Minimum sustained idle duration in minutes before virtualizing a link.
    pub idle_min_minutes: u64,
    /// RTT sample age in seconds at/above which RTT is treated as missing/unsafe.
    pub rtt_missing_seconds: u64,
    /// Utilization percentage above which a virtual link should be unvirtualized.
    pub unvirtualize_util_pct: f32,
    /// Minimum dwell time in minutes before a node may change state again.
    pub min_state_dwell_minutes: u64,
    /// Maximum number of link state changes per hour.
    pub max_link_changes_per_hour: u32,
    /// Cooldown in minutes between topology reload attempts.
    pub reload_cooldown_minutes: u64,
    /// Whether TreeGuard should automatically virtualize top-level nodes (root object keys in
    /// `network.json`) when they appear safe.
    pub top_level_auto_virtualize: bool,
    /// Utilization percentage below which a top-level node is considered safe to virtualize
    /// (sustained for a fixed window).
    pub top_level_safe_util_pct: f32,
}

impl Default for TreeguardLinksConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            all_nodes: false,
            nodes: Vec::new(),
            idle_util_pct: 2.0,
            idle_min_minutes: 15,
            rtt_missing_seconds: 120,
            unvirtualize_util_pct: 5.0,
            min_state_dwell_minutes: 30,
            max_link_changes_per_hour: 4,
            reload_cooldown_minutes: 10,
            top_level_auto_virtualize: true,
            top_level_safe_util_pct: 85.0,
        }
    }
}

/// TreeGuard circuit SQM switching configuration.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Allocative)]
#[serde(default)]
pub struct TreeguardCircuitsConfig {
    /// Whether per-circuit management is enabled.
    pub enabled: bool,
    /// Whether TreeGuard may manage all circuits found in ShapedDevices.
    ///
    /// When enabled, the `circuits` allowlist is ignored.
    #[serde(default)]
    pub all_circuits: bool,
    /// Circuit allowlist: circuit IDs (strings, as in ShapedDevices.csv) that TreeGuard may manage.
    pub circuits: Vec<String>,
    /// Whether SQM switching is enabled.
    pub switching_enabled: bool,
    /// Whether TreeGuard may make independent decisions for down vs up directions.
    pub independent_directions: bool,
    /// Utilization percentage below which a circuit direction is considered idle.
    pub idle_util_pct: f32,
    /// Minimum sustained idle duration in minutes before downgrading SQM for a direction.
    pub idle_min_minutes: u64,
    /// RTT sample age in seconds at/above which RTT is treated as missing/unsafe.
    pub rtt_missing_seconds: u64,
    /// Utilization percentage above which a downgraded direction should be upgraded back to CAKE.
    pub upgrade_util_pct: f32,
    /// Minimum dwell time in minutes before a circuit may switch again.
    pub min_switch_dwell_minutes: u64,
    /// Maximum number of SQM switches per hour.
    pub max_switches_per_hour: u32,
    /// Whether TreeGuard should persist SQM overrides to avoid scheduler fights.
    pub persist_sqm_overrides: bool,
}

impl Default for TreeguardCircuitsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            all_circuits: false,
            circuits: Vec::new(),
            switching_enabled: true,
            independent_directions: true,
            idle_util_pct: 2.0,
            idle_min_minutes: 15,
            rtt_missing_seconds: 120,
            upgrade_util_pct: 5.0,
            min_switch_dwell_minutes: 30,
            max_switches_per_hour: 4,
            persist_sqm_overrides: true,
        }
    }
}

/// TreeGuard QoO guardrail configuration.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Allocative)]
#[serde(default)]
pub struct TreeguardQooConfig {
    /// Whether QoO guardrails are enabled.
    pub enabled: bool,
    /// Minimum QoO score (0..100) required for TreeGuard to take CPU-saving actions when QoO is available.
    pub min_score: f32,
}

impl Default for TreeguardQooConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_score: 80.0,
        }
    }
}
