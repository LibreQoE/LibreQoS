//! TreeGuard configuration definitions.

use allocative::Allocative;
use serde::{Deserialize, Serialize};

fn default_enabled() -> bool {
    true
}

fn default_false() -> bool {
    false
}

fn default_tick_seconds() -> u64 {
    1
}

fn default_cpu_high_pct() -> u8 {
    75
}

fn default_cpu_low_pct() -> u8 {
    55
}

fn default_idle_util_pct() -> f32 {
    2.0
}

fn default_idle_min_minutes() -> u32 {
    15
}

fn default_rtt_missing_seconds() -> u32 {
    120
}

fn default_unvirtualize_util_pct() -> f32 {
    5.0
}

fn default_min_state_dwell_minutes() -> u32 {
    30
}

fn default_max_link_changes_per_hour() -> u32 {
    4
}

fn default_reload_cooldown_minutes() -> u32 {
    10
}

fn default_top_level_safe_util_pct() -> f32 {
    85.0
}

fn default_upgrade_util_pct() -> f32 {
    5.0
}

fn default_min_switch_dwell_minutes() -> u32 {
    30
}

fn default_max_switches_per_hour() -> u32 {
    4
}

fn default_min_score() -> f32 {
    70.0
}

/// CPU modes supported by TreeGuard.
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq, Default, Allocative)]
#[serde(rename_all = "snake_case")]
pub enum TreeguardCpuMode {
    /// Consider CPU thresholds when making decisions.
    #[default]
    CpuAware,
    /// Use only traffic and RTT signals.
    TrafficRttOnly,
}

/// Top-level TreeGuard configuration.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Allocative)]
#[serde(default)]
pub struct TreeguardConfig {
    /// Enables TreeGuard globally.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// If true, TreeGuard records intended actions but does not apply them.
    #[serde(default = "default_false")]
    pub dry_run: bool,
    /// Decision cadence.
    #[serde(default = "default_tick_seconds")]
    pub tick_seconds: u64,
    /// CPU guardrail settings.
    pub cpu: TreeguardCpuConfig,
    /// Link/node virtualization settings.
    pub links: TreeguardLinksConfig,
    /// Circuit SQM switching settings.
    pub circuits: TreeguardCircuitsConfig,
    /// QoO guardrail settings.
    pub qoo: TreeguardQooConfig,
}

impl Default for TreeguardConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            dry_run: default_false(),
            tick_seconds: default_tick_seconds(),
            cpu: TreeguardCpuConfig::default(),
            links: TreeguardLinksConfig::default(),
            circuits: TreeguardCircuitsConfig::default(),
            qoo: TreeguardQooConfig::default(),
        }
    }
}

/// TreeGuard CPU guardrails.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Allocative)]
#[serde(default)]
pub struct TreeguardCpuConfig {
    /// How CPU participates in decision making.
    pub mode: TreeguardCpuMode,
    /// High watermark for CPU pressure.
    #[serde(default = "default_cpu_high_pct")]
    pub cpu_high_pct: u8,
    /// Low watermark for reverting CPU-saving actions.
    #[serde(default = "default_cpu_low_pct")]
    pub cpu_low_pct: u8,
}

impl Default for TreeguardCpuConfig {
    fn default() -> Self {
        Self {
            mode: TreeguardCpuMode::default(),
            cpu_high_pct: default_cpu_high_pct(),
            cpu_low_pct: default_cpu_low_pct(),
        }
    }
}

/// TreeGuard node virtualization settings.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Allocative)]
#[serde(default)]
pub struct TreeguardLinksConfig {
    /// Enables TreeGuard management of node virtualization.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Apply to all eligible nodes instead of an allowlist.
    #[serde(default = "default_enabled")]
    pub all_nodes: bool,
    /// Explicit allowlist when `all_nodes` is false.
    pub nodes: Vec<String>,
    /// Utilization threshold below which a link is considered idle.
    #[serde(default = "default_idle_util_pct")]
    pub idle_util_pct: f32,
    /// Required duration for idle classification.
    #[serde(default = "default_idle_min_minutes")]
    pub idle_min_minutes: u32,
    /// RTT freshness timeout.
    #[serde(default = "default_rtt_missing_seconds")]
    pub rtt_missing_seconds: u32,
    /// Utilization threshold to unvirtualize.
    #[serde(default = "default_unvirtualize_util_pct")]
    pub unvirtualize_util_pct: f32,
    /// Minimum state dwell before another change.
    #[serde(default = "default_min_state_dwell_minutes")]
    pub min_state_dwell_minutes: u32,
    /// Rate limit on link changes.
    #[serde(default = "default_max_link_changes_per_hour")]
    pub max_link_changes_per_hour: u32,
    /// Cooldown between topology reloads.
    #[serde(default = "default_reload_cooldown_minutes")]
    pub reload_cooldown_minutes: u32,
    /// Allow TreeGuard to virtualize top-level nodes.
    #[serde(default = "default_enabled")]
    pub top_level_auto_virtualize: bool,
    /// Utilization ceiling for safe top-level virtualization.
    #[serde(default = "default_top_level_safe_util_pct")]
    pub top_level_safe_util_pct: f32,
}

impl Default for TreeguardLinksConfig {
    fn default() -> Self {
        Self {
            enabled: default_false(),
            all_nodes: default_enabled(),
            nodes: Vec::new(),
            idle_util_pct: default_idle_util_pct(),
            idle_min_minutes: default_idle_min_minutes(),
            rtt_missing_seconds: default_rtt_missing_seconds(),
            unvirtualize_util_pct: default_unvirtualize_util_pct(),
            min_state_dwell_minutes: default_min_state_dwell_minutes(),
            max_link_changes_per_hour: default_max_link_changes_per_hour(),
            reload_cooldown_minutes: default_reload_cooldown_minutes(),
            top_level_auto_virtualize: default_false(),
            top_level_safe_util_pct: default_top_level_safe_util_pct(),
        }
    }
}

/// TreeGuard circuit management settings.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Allocative)]
#[serde(default)]
pub struct TreeguardCircuitsConfig {
    /// Enables TreeGuard circuit management.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Apply to all circuits instead of an allowlist.
    #[serde(default = "default_enabled")]
    pub all_circuits: bool,
    /// Explicit allowlist when `all_circuits` is false.
    pub circuits: Vec<String>,
    /// Enables SQM switching actions.
    #[serde(default = "default_enabled")]
    pub switching_enabled: bool,
    /// Manage upload/download directions independently.
    #[serde(default = "default_enabled")]
    pub independent_directions: bool,
    /// Utilization threshold below which a circuit is considered idle.
    #[serde(default = "default_idle_util_pct")]
    pub idle_util_pct: f32,
    /// Required duration for idle classification.
    #[serde(default = "default_idle_min_minutes")]
    pub idle_min_minutes: u32,
    /// RTT freshness timeout.
    #[serde(default = "default_rtt_missing_seconds")]
    pub rtt_missing_seconds: u32,
    /// Utilization threshold to switch back to CAKE.
    #[serde(default = "default_upgrade_util_pct")]
    pub upgrade_util_pct: f32,
    /// Minimum dwell before another SQM switch.
    #[serde(default = "default_min_switch_dwell_minutes")]
    pub min_switch_dwell_minutes: u32,
    /// Rate limit on SQM switches.
    #[serde(default = "default_max_switches_per_hour")]
    pub max_switches_per_hour: u32,
    /// Persist overrides to avoid scheduler fights.
    #[serde(default = "default_enabled")]
    pub persist_sqm_overrides: bool,
}

impl Default for TreeguardCircuitsConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            all_circuits: default_enabled(),
            circuits: Vec::new(),
            switching_enabled: default_enabled(),
            independent_directions: default_enabled(),
            idle_util_pct: default_idle_util_pct(),
            idle_min_minutes: default_idle_min_minutes(),
            rtt_missing_seconds: default_rtt_missing_seconds(),
            upgrade_util_pct: default_upgrade_util_pct(),
            min_switch_dwell_minutes: default_min_switch_dwell_minutes(),
            max_switches_per_hour: default_max_switches_per_hour(),
            persist_sqm_overrides: default_enabled(),
        }
    }
}

/// TreeGuard QoO protection settings.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Allocative)]
#[serde(default)]
pub struct TreeguardQooConfig {
    /// Enables QoO protection.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Minimum QoO score required to take CPU-saving actions.
    #[serde(default = "default_min_score")]
    pub min_score: f32,
}

impl Default for TreeguardQooConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            min_score: default_min_score(),
        }
    }
}

impl TreeguardConfig {
    /// Validates TreeGuard configuration values and cross-field relationships.
    pub fn validate(&self) -> Result<(), String> {
        validate_percent_u8("treeguard.cpu.cpu_high_pct", self.cpu.cpu_high_pct)?;
        validate_percent_u8("treeguard.cpu.cpu_low_pct", self.cpu.cpu_low_pct)?;
        if self.cpu.cpu_low_pct > self.cpu.cpu_high_pct {
            return Err(
                "treeguard.cpu.cpu_low_pct must be less than or equal to treeguard.cpu.cpu_high_pct"
                    .to_string(),
            );
        }

        validate_non_zero("treeguard.tick_seconds", self.tick_seconds)?;

        self.links.validate()?;
        self.circuits.validate()?;
        self.qoo.validate()?;

        Ok(())
    }
}

impl TreeguardLinksConfig {
    fn validate(&self) -> Result<(), String> {
        validate_percent_f32("treeguard.links.idle_util_pct", self.idle_util_pct)?;
        validate_percent_f32(
            "treeguard.links.unvirtualize_util_pct",
            self.unvirtualize_util_pct,
        )?;
        validate_percent_f32(
            "treeguard.links.top_level_safe_util_pct",
            self.top_level_safe_util_pct,
        )?;
        Ok(())
    }
}

impl TreeguardCircuitsConfig {
    fn validate(&self) -> Result<(), String> {
        validate_percent_f32("treeguard.circuits.idle_util_pct", self.idle_util_pct)?;
        validate_percent_f32("treeguard.circuits.upgrade_util_pct", self.upgrade_util_pct)?;
        if self.upgrade_util_pct < self.idle_util_pct {
            return Err(
                "treeguard.circuits.upgrade_util_pct must be greater than or equal to treeguard.circuits.idle_util_pct"
                    .to_string(),
            );
        }
        Ok(())
    }
}

impl TreeguardQooConfig {
    fn validate(&self) -> Result<(), String> {
        validate_percent_f32("treeguard.qoo.min_score", self.min_score)
    }
}

fn validate_percent_u8(name: &str, value: u8) -> Result<(), String> {
    if value > 100 {
        return Err(format!("{name} must be between 0 and 100"));
    }
    Ok(())
}

fn validate_percent_f32(name: &str, value: f32) -> Result<(), String> {
    if !value.is_finite() || !(0.0..=100.0).contains(&value) {
        return Err(format!("{name} must be between 0 and 100"));
    }
    Ok(())
}

fn validate_non_zero(name: &str, value: u64) -> Result<(), String> {
    if value == 0 {
        return Err(format!("{name} must be greater than 0"));
    }
    Ok(())
}
