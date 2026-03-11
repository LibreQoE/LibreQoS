//! StormGuard definitions (originally from ispConfig.py)

use allocative::Allocative;
use serde::{Deserialize, Serialize};

fn default_false() -> bool {
    false
}

fn default_true() -> bool {
    true
}

fn default_stormguard_strategy() -> StormguardStrategy {
    StormguardStrategy::DelayProbe
}

fn default_minimum_pct() -> f32 {
    0.5
}

fn default_increase_fast_multiplier() -> f32 {
    1.30
}

fn default_increase_multiplier() -> f32 {
    1.15
}

fn default_decrease_multiplier() -> f32 {
    0.95
}

fn default_decrease_fast_multiplier() -> f32 {
    0.88
}

fn default_increase_fast_cooldown_seconds() -> f32 {
    2.0
}

fn default_increase_cooldown_seconds() -> f32 {
    1.0
}

fn default_decrease_cooldown_seconds() -> f32 {
    3.75
}

fn default_decrease_fast_cooldown_seconds() -> f32 {
    7.5
}

fn default_fallback_sqm() -> String {
    "fq_codel".to_string()
}

fn default_delay_threshold_ms() -> f32 {
    40.0
}

fn default_delay_threshold_ratio() -> f32 {
    1.10
}

fn default_baseline_alpha_up() -> f32 {
    0.01
}

fn default_baseline_alpha_down() -> f32 {
    0.10
}

fn default_probe_interval_seconds() -> f32 {
    10.0
}

fn default_min_throughput_mbps_for_rtt() -> f32 {
    0.05
}

fn default_active_ping_target() -> String {
    "1.1.1.1".to_string()
}

fn default_active_ping_interval_seconds() -> f32 {
    10.0
}

fn default_active_ping_weight() -> f32 {
    0.70
}

fn default_active_ping_timeout_seconds() -> f32 {
    1.0
}

/// StormGuard evaluation strategy.
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq, Allocative)]
#[serde(rename_all = "snake_case")]
pub enum StormguardStrategy {
    /// Legacy score-based evaluation.
    LegacyScore,
    /// CAKE-autorate-inspired delay baseline + probing strategy.
    DelayProbe,
    /// DelayProbe + infrequent active ICMP ping RTT sampling.
    DelayProbeActive,
}

/// Configuration for the StormGuard module (auto-rate).
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Allocative)]
#[serde(default)]
pub struct StormguardConfig {
    /// Whether StormGuard is enabled.
    #[serde(default = "default_false")]
    pub enabled: bool,
    /// Apply to all eligible top-level sites in the queue structure.
    #[serde(default = "default_false")]
    pub all_sites: bool,
    /// Optional site allowlist when `all_sites = false`.
    pub targets: Vec<String>,
    /// Optional site exclusion list, primarily for `all_sites = true`.
    pub exclude_sites: Vec<String>,
    /// Whether to run in dry run mode (no actual changes).
    #[serde(default = "default_true")]
    pub dry_run: bool,
    /// Optional log file path - emits a CSV of site and rates.
    pub log_file: Option<String>,
    /// Evaluation strategy (legacy scoring or delay-probe).
    #[serde(default = "default_stormguard_strategy")]
    pub strategy: StormguardStrategy,
    /// Minimum percentage (e.g. 0.5 for 50%) download.
    #[serde(default = "default_minimum_pct")]
    pub minimum_download_percentage: f32,
    /// Minimum percentage (e.g. 0.5 for 50%) upload.
    #[serde(default = "default_minimum_pct")]
    pub minimum_upload_percentage: f32,
    /// Multiplier used for aggressive increase actions.
    #[serde(default = "default_increase_fast_multiplier")]
    pub increase_fast_multiplier: f32,
    /// Multiplier used for normal increase actions.
    #[serde(default = "default_increase_multiplier")]
    pub increase_multiplier: f32,
    /// Multiplier used for normal decrease actions.
    #[serde(default = "default_decrease_multiplier")]
    pub decrease_multiplier: f32,
    /// Multiplier used for aggressive decrease actions.
    #[serde(default = "default_decrease_fast_multiplier")]
    pub decrease_fast_multiplier: f32,
    /// Cooldown applied after aggressive increase actions.
    #[serde(default = "default_increase_fast_cooldown_seconds")]
    pub increase_fast_cooldown_seconds: f32,
    /// Cooldown applied after normal increase actions.
    #[serde(default = "default_increase_cooldown_seconds")]
    pub increase_cooldown_seconds: f32,
    /// Cooldown applied after normal decrease actions.
    #[serde(default = "default_decrease_cooldown_seconds")]
    pub decrease_cooldown_seconds: f32,
    /// Cooldown applied after aggressive decrease actions.
    #[serde(default = "default_decrease_fast_cooldown_seconds")]
    pub decrease_fast_cooldown_seconds: f32,
    /// Whether StormGuard may fall back to a per-circuit SQM action when direct HTB changes are unsafe.
    #[serde(default = "default_false")]
    pub circuit_fallback_enabled: bool,
    /// Whether fallback SQM actions should be persisted into the StormGuard override layer.
    #[serde(default = "default_true")]
    pub circuit_fallback_persist: bool,
    /// SQM token to apply during fallback (for now, `fq_codel` or `cake`).
    #[serde(default = "default_fallback_sqm")]
    pub circuit_fallback_sqm: String,

    // --- DelayProbe strategy knobs (safe to keep set even in legacy mode) ---
    /// Standing-delay threshold in milliseconds that triggers a decrease (DelayProbe).
    #[serde(default = "default_delay_threshold_ms")]
    pub delay_threshold_ms: f32,
    /// Standing-delay threshold as a ratio over baseline RTT (DelayProbe).
    #[serde(default = "default_delay_threshold_ratio")]
    pub delay_threshold_ratio: f32,
    /// Baseline RTT EWMA alpha used when RTT is rising (baseline drifts up slowly).
    #[serde(default = "default_baseline_alpha_up")]
    pub baseline_alpha_up: f32,
    /// Baseline RTT EWMA alpha used when RTT is falling (baseline tracks down quickly).
    #[serde(default = "default_baseline_alpha_down")]
    pub baseline_alpha_down: f32,
    /// Minimum time between probe-style increases (DelayProbe).
    #[serde(default = "default_probe_interval_seconds")]
    pub probe_interval_seconds: f32,
    /// Minimum throughput (Mbps) required before RTT-based decisions are trusted (DelayProbe).
    #[serde(default = "default_min_throughput_mbps_for_rtt")]
    pub min_throughput_mbps_for_rtt: f32,

    // --- DelayProbeActive knobs (safe to keep set even in other modes) ---
    /// Target hostname or IP for active RTT sampling (DelayProbeActive).
    #[serde(default = "default_active_ping_target")]
    pub active_ping_target: String,
    /// Interval between active pings (DelayProbeActive).
    #[serde(default = "default_active_ping_interval_seconds")]
    pub active_ping_interval_seconds: f32,
    /// Weight for active ping RTT when blending with passive RTT (0..=1, DelayProbeActive).
    #[serde(default = "default_active_ping_weight")]
    pub active_ping_weight: f32,
    /// Timeout for active pings (seconds, DelayProbeActive).
    #[serde(default = "default_active_ping_timeout_seconds")]
    pub active_ping_timeout_seconds: f32,
}

impl Default for StormguardConfig {
    fn default() -> Self {
        Self {
            enabled: default_false(),
            all_sites: default_false(),
            targets: Vec::new(),
            exclude_sites: Vec::new(),
            dry_run: default_true(),
            log_file: None,
            strategy: default_stormguard_strategy(),
            minimum_download_percentage: default_minimum_pct(),
            minimum_upload_percentage: default_minimum_pct(),
            increase_fast_multiplier: default_increase_fast_multiplier(),
            increase_multiplier: default_increase_multiplier(),
            decrease_multiplier: default_decrease_multiplier(),
            decrease_fast_multiplier: default_decrease_fast_multiplier(),
            increase_fast_cooldown_seconds: default_increase_fast_cooldown_seconds(),
            increase_cooldown_seconds: default_increase_cooldown_seconds(),
            decrease_cooldown_seconds: default_decrease_cooldown_seconds(),
            decrease_fast_cooldown_seconds: default_decrease_fast_cooldown_seconds(),
            circuit_fallback_enabled: default_false(),
            circuit_fallback_persist: default_true(),
            circuit_fallback_sqm: default_fallback_sqm(),
            delay_threshold_ms: default_delay_threshold_ms(),
            delay_threshold_ratio: default_delay_threshold_ratio(),
            baseline_alpha_up: default_baseline_alpha_up(),
            baseline_alpha_down: default_baseline_alpha_down(),
            probe_interval_seconds: default_probe_interval_seconds(),
            min_throughput_mbps_for_rtt: default_min_throughput_mbps_for_rtt(),
            active_ping_target: default_active_ping_target(),
            active_ping_interval_seconds: default_active_ping_interval_seconds(),
            active_ping_weight: default_active_ping_weight(),
            active_ping_timeout_seconds: default_active_ping_timeout_seconds(),
        }
    }
}

impl StormguardConfig {
    /// Validates StormGuard configuration values and relationships.
    pub fn validate(&self) -> Result<(), String> {
        if self.enabled && !self.all_sites && self.targets.is_empty() {
            return Err(
                "stormguard.targets must not be empty when stormguard.enabled = true and stormguard.all_sites = false"
                    .to_string(),
            );
        }

        validate_percentage(
            "stormguard.minimum_download_percentage",
            self.minimum_download_percentage,
        )?;
        validate_percentage(
            "stormguard.minimum_upload_percentage",
            self.minimum_upload_percentage,
        )?;

        validate_multiplier_gt_one(
            "stormguard.increase_fast_multiplier",
            self.increase_fast_multiplier,
        )?;
        validate_multiplier_gt_one(
            "stormguard.increase_multiplier",
            self.increase_multiplier,
        )?;
        validate_multiplier_lt_one(
            "stormguard.decrease_multiplier",
            self.decrease_multiplier,
        )?;
        validate_multiplier_lt_one(
            "stormguard.decrease_fast_multiplier",
            self.decrease_fast_multiplier,
        )?;

        validate_positive_seconds(
            "stormguard.increase_fast_cooldown_seconds",
            self.increase_fast_cooldown_seconds,
        )?;
        validate_positive_seconds(
            "stormguard.increase_cooldown_seconds",
            self.increase_cooldown_seconds,
        )?;
        validate_positive_seconds(
            "stormguard.decrease_cooldown_seconds",
            self.decrease_cooldown_seconds,
        )?;
        validate_positive_seconds(
            "stormguard.decrease_fast_cooldown_seconds",
            self.decrease_fast_cooldown_seconds,
        )?;

        let sqm = self.circuit_fallback_sqm.trim().to_ascii_lowercase();
        if self.circuit_fallback_enabled && !matches!(sqm.as_str(), "fq_codel" | "cake") {
            return Err(
                "stormguard.circuit_fallback_sqm must be either 'fq_codel' or 'cake'"
                    .to_string(),
            );
        }

        validate_positive_seconds("stormguard.probe_interval_seconds", self.probe_interval_seconds)?;
        validate_positive("stormguard.delay_threshold_ms", self.delay_threshold_ms)?;
        validate_ratio_gt_one(
            "stormguard.delay_threshold_ratio",
            self.delay_threshold_ratio,
        )?;
        validate_alpha("stormguard.baseline_alpha_up", self.baseline_alpha_up)?;
        validate_alpha("stormguard.baseline_alpha_down", self.baseline_alpha_down)?;
        validate_nonnegative(
            "stormguard.min_throughput_mbps_for_rtt",
            self.min_throughput_mbps_for_rtt,
        )?;

        validate_nonnegative("stormguard.active_ping_weight", self.active_ping_weight)?;
        if self.active_ping_weight > 1.0 {
            return Err("stormguard.active_ping_weight must be <= 1.0".to_string());
        }

        validate_positive_seconds(
            "stormguard.active_ping_interval_seconds",
            self.active_ping_interval_seconds,
        )?;
        validate_positive_seconds(
            "stormguard.active_ping_timeout_seconds",
            self.active_ping_timeout_seconds,
        )?;
        if matches!(self.strategy, StormguardStrategy::DelayProbeActive)
            && self.active_ping_target.trim().is_empty()
        {
            return Err(
                "stormguard.active_ping_target must not be empty when strategy = delay_probe_active"
                    .to_string(),
            );
        }

        Ok(())
    }
}

fn validate_percentage(name: &str, value: f32) -> Result<(), String> {
    if !(0.0..=1.0).contains(&value) || value <= 0.0 {
        return Err(format!("{name} must be > 0.0 and <= 1.0"));
    }
    Ok(())
}

fn validate_multiplier_gt_one(name: &str, value: f32) -> Result<(), String> {
    if !value.is_finite() || value <= 1.0 {
        return Err(format!("{name} must be > 1.0"));
    }
    Ok(())
}

fn validate_multiplier_lt_one(name: &str, value: f32) -> Result<(), String> {
    if !value.is_finite() || !(0.0..1.0).contains(&value) {
        return Err(format!("{name} must be > 0.0 and < 1.0"));
    }
    Ok(())
}

fn validate_positive_seconds(name: &str, value: f32) -> Result<(), String> {
    if !value.is_finite() || value <= 0.0 {
        return Err(format!("{name} must be > 0.0"));
    }
    Ok(())
}

fn validate_positive(name: &str, value: f32) -> Result<(), String> {
    if !value.is_finite() || value <= 0.0 {
        return Err(format!("{name} must be > 0.0"));
    }
    Ok(())
}

fn validate_ratio_gt_one(name: &str, value: f32) -> Result<(), String> {
    if !value.is_finite() || value <= 1.0 {
        return Err(format!("{name} must be > 1.0"));
    }
    Ok(())
}

fn validate_alpha(name: &str, value: f32) -> Result<(), String> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) || value == 0.0 {
        return Err(format!("{name} must be > 0.0 and <= 1.0"));
    }
    Ok(())
}

fn validate_nonnegative(name: &str, value: f32) -> Result<(), String> {
    if !value.is_finite() || value < 0.0 {
        return Err(format!("{name} must be >= 0.0"));
    }
    Ok(())
}
