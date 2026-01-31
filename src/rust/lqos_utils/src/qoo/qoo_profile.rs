//! JSON-serializable QoO profile table (ship defaults + user-editable).
//!
//! This file defines a *wire format* that is easy for users to edit, validates it strictly, and
//! converts entries into the runtime `QooProfile` used by `compute_qoo()`.
//!
//! Recommended workflow:
//! - Ship a default `profiles.json` with LibreQoS.
//! - Load and validate it on startup.
//! - Allow UI edits and save back to disk (pretty JSON).
//!
//! The JSON format intentionally uses:
//! - Mbps for throughput
//! - milliseconds for latency thresholds
//! - percent for loss thresholds (0..100)

use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

use crate::qoo::{
    Baseline, CombineMode, LatencyNormalization, LatencyReq, LossHandling, LowHigh, QooProfile,
};
use crate::rtt::RttBucket;

/// File containing a table of QoO profiles.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QooProfilesFile {
    pub schema_version: u32,

    #[serde(default)]
    pub default_profile_id: Option<String>,

    pub profiles: Vec<QooProfileSpec>,
}

impl QooProfilesFile {
    pub fn load_json<P: AsRef<Path>>(path: P) -> Result<Self, ProfileIoError> {
        let s = fs::read_to_string(path)?;
        let doc: Self = serde_json::from_str(&s)?;
        doc.validate()?;
        Ok(doc)
    }

    pub fn save_json_pretty<P: AsRef<Path>>(&self, path: P) -> Result<(), ProfileIoError> {
        self.validate()?;
        let s = serde_json::to_string_pretty(self)?;
        fs::write(path, s)?;
        Ok(())
    }

    /// Strict validation (fail fast) so a user can’t silently create nonsense.
    pub fn validate(&self) -> Result<(), ProfileIoError> {
        if self.schema_version != 1 {
            return Err(ProfileIoError::Validation(vec![format!(
                "Unsupported schema_version {} (expected 1)",
                self.schema_version
            )]));
        }

        let mut errs: Vec<String> = Vec::new();

        // Ensure IDs are unique.
        {
            use std::collections::HashSet;
            let mut seen = HashSet::new();
            for p in &self.profiles {
                if !seen.insert(&p.id) {
                    errs.push(format!("Duplicate profile id '{}'", p.id));
                }
            }
        }

        // Default profile must exist (if set).
        if let Some(default_id) = &self.default_profile_id {
            if !self.profiles.iter().any(|p| &p.id == default_id) {
                errs.push(format!(
                    "default_profile_id '{}' does not match any profile id",
                    default_id
                ));
            }
        }

        for p in &self.profiles {
            errs.extend(p.validate().into_iter().map(|e| format!("[{}] {e}", p.id)));
        }

        if errs.is_empty() {
            Ok(())
        } else {
            Err(ProfileIoError::Validation(errs))
        }
    }

    /// Find the default profile (if configured), otherwise first profile.
    pub fn pick_default(&self) -> Option<&QooProfileSpec> {
        if let Some(id) = &self.default_profile_id {
            if let Some(p) = self.profiles.iter().find(|p| &p.id == id) {
                return Some(p);
            }
        }
        self.profiles.first()
    }
}

#[derive(Debug)]
pub enum ProfileIoError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Validation(Vec<String>),
}

impl std::fmt::Display for ProfileIoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProfileIoError::Io(e) => write!(f, "I/O error: {e}"),
            ProfileIoError::Json(e) => write!(f, "JSON error: {e}"),
            ProfileIoError::Validation(errs) => {
                writeln!(f, "Profile validation failed:")?;
                for e in errs {
                    writeln!(f, "  - {e}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for ProfileIoError {}

impl From<std::io::Error> for ProfileIoError {
    fn from(e: std::io::Error) -> Self {
        ProfileIoError::Io(e)
    }
}

impl From<serde_json::Error> for ProfileIoError {
    fn from(e: serde_json::Error) -> Self {
        ProfileIoError::Json(e)
    }
}

/// Editable-on-disk profile (wire format).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QooProfileSpec {
    pub id: String,
    pub name: String,

    #[serde(default)]
    pub description: Option<String>,

    #[serde(default)]
    pub rtt_scope: RttScopeSpec,

    /// Throughput thresholds in Mbps.
    pub download_mbps: Range,
    pub upload_mbps: Range,

    /// Usually just [{percentile: 95, ...}] for LibreQoS parity.
    pub latency: Vec<LatencySpec>,

    /// Percent (0..100).
    pub loss_percent: Range,

    /// Optional latency bias/normalization.
    #[serde(default)]
    pub latency_normalization: LatencyNormalizationSpec,

    #[serde(default)]
    pub combine: CombineModeSpec,

    #[serde(default)]
    pub loss_handling: LossHandlingSpec,
}

impl QooProfileSpec {
    /// Convert this wire-format profile into the runtime `QooProfile`.
    pub fn to_runtime(&self) -> QooProfile {
        let rtt_scope = match self.rtt_scope {
            RttScopeSpec::Current => RttBucket::Current,
            RttScopeSpec::Total => RttBucket::Total,
        };

        let latency = self
            .latency
            .iter()
            .map(|l| LatencyReq {
                percentile: l.percentile,
                rtt_ms: LowHigh::lower_is_better(l.low_ms, l.high_ms),
            })
            .collect();

        let loss_fraction = LowHigh::lower_is_better(
            (self.loss_percent.low / 100.0).clamp(0.0, 1.0),
            (self.loss_percent.high / 100.0).clamp(0.0, 1.0),
        );

        let combine = match self.combine {
            CombineModeSpec::IetfLatencyAndLoss => CombineMode::IetfLatencyAndLoss,
            CombineModeSpec::LibreqosLatencyLossThroughput => CombineMode::LibreQosLatencyLossThroughput,
        };

        let loss_handling = match self.loss_handling {
            LossHandlingSpec::Strict => LossHandling::Strict,
            LossHandlingSpec::ConfidenceWeighted => LossHandling::ConfidenceWeighted,
        };

        let latency_normalization = match self.latency_normalization {
            LatencyNormalizationSpec::None => LatencyNormalization::None,
            LatencyNormalizationSpec::ThresholdOffsetMs { ms } => {
                LatencyNormalization::ThresholdOffsetMs { ms }
            }
            LatencyNormalizationSpec::ExcessOverBaseline { baseline } => {
                let b = match baseline {
                    BaselineSpec::FixedMs { ms } => Baseline::FixedMs { ms },
                    BaselineSpec::Percentile { percentile } => Baseline::Percentile { percentile },
                };
                LatencyNormalization::ExcessOverBaseline { baseline: b }
            }
        };

        QooProfile {
            name: self.name.clone(),
            rtt_scope,

            download_mbps: LowHigh::higher_is_better(self.download_mbps.low, self.download_mbps.high),
            upload_mbps: LowHigh::higher_is_better(self.upload_mbps.low, self.upload_mbps.high),

            latency,
            loss_fraction,

            combine,
            loss_handling,

            latency_normalization,
        }
    }

    fn validate(&self) -> Vec<String> {
        let mut errs = Vec::new();

        if self.id.trim().is_empty() {
            errs.push("id must not be empty".into());
        }
        if self.name.trim().is_empty() {
            errs.push("name must not be empty".into());
        }

        // Throughput ranges: higher-is-better, so low <= high.
        if !(self.download_mbps.low.is_finite() && self.download_mbps.high.is_finite()) {
            errs.push("download_mbps values must be finite".into());
        } else if self.download_mbps.low < 0.0 || self.download_mbps.high < 0.0 {
            errs.push("download_mbps must be >= 0".into());
        } else if self.download_mbps.low > self.download_mbps.high {
            errs.push("download_mbps.low must be <= download_mbps.high".into());
        }

        if !(self.upload_mbps.low.is_finite() && self.upload_mbps.high.is_finite()) {
            errs.push("upload_mbps values must be finite".into());
        } else if self.upload_mbps.low < 0.0 || self.upload_mbps.high < 0.0 {
            errs.push("upload_mbps must be >= 0".into());
        } else if self.upload_mbps.low > self.upload_mbps.high {
            errs.push("upload_mbps.low must be <= upload_mbps.high".into());
        }

        // Loss in percent.
        if self.loss_percent.low < 0.0
            || self.loss_percent.high < 0.0
            || self.loss_percent.low > 100.0
            || self.loss_percent.high > 100.0
        {
            errs.push("loss_percent must be within 0..100".into());
        }

        if self.latency.is_empty() {
            errs.push("latency must contain at least one percentile entry (e.g. p95)".into());
        }

        for l in &self.latency {
            if l.percentile > 100 {
                errs.push(format!("latency percentile {} must be 0..100", l.percentile));
            }
            if !l.low_ms.is_finite() || !l.high_ms.is_finite() {
                errs.push(format!("latency p{} values must be finite", l.percentile));
            } else if l.low_ms < 0.0 || l.high_ms < 0.0 {
                errs.push(format!("latency p{} values must be >= 0", l.percentile));
            }
        }

        // Normalization sanity.
        match self.latency_normalization {
            LatencyNormalizationSpec::None => {}
            LatencyNormalizationSpec::ThresholdOffsetMs { ms } => {
                if !ms.is_finite() || ms < 0.0 {
                    errs.push("latency_normalization.threshold_offset_ms.ms must be finite and >= 0".into());
                }
            }
            LatencyNormalizationSpec::ExcessOverBaseline { baseline } => match baseline {
                BaselineSpec::FixedMs { ms } => {
                    if !ms.is_finite() || ms < 0.0 {
                        errs.push("latency_normalization.excess_over_baseline.fixed_ms.ms must be finite and >= 0".into());
                    }
                }
                BaselineSpec::Percentile { percentile } => {
                    if percentile > 100 {
                        errs.push("latency_normalization.excess_over_baseline.percentile must be 0..100".into());
                    }
                }
            },
        }

        errs
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RttScopeSpec {
    Current,
    Total,
}

impl Default for RttScopeSpec {
    fn default() -> Self {
        RttScopeSpec::Current
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Range {
    pub low: f64,
    pub high: f64,
}

/// One latency percentile line item.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LatencySpec {
    pub percentile: u8,

    /// "Bad/minimum/unacceptable" threshold (ms)
    pub low_ms: f64,

    /// "Good/target/optimal" threshold (ms)
    pub high_ms: f64,
}

/// Latency bias / normalization options.
///
/// - `threshold_offset_ms`: Subtract a fixed `ms` before scoring (equivalent to shifting thresholds upward).
/// - `excess_over_baseline`: Subtract a baseline derived from the same RTT distribution (or fixed).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum LatencyNormalizationSpec {
    None,
    ThresholdOffsetMs { ms: f64 },
    ExcessOverBaseline { baseline: BaselineSpec },
}

impl Default for LatencyNormalizationSpec {
    fn default() -> Self {
        LatencyNormalizationSpec::None
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BaselineSpec {
    FixedMs { ms: f64 },
    Percentile { percentile: u8 },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CombineModeSpec {
    IetfLatencyAndLoss,
    LibreqosLatencyLossThroughput,
}

impl Default for CombineModeSpec {
    fn default() -> Self {
        CombineModeSpec::IetfLatencyAndLoss
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LossHandlingSpec {
    Strict,
    ConfidenceWeighted,
}

impl Default for LossHandlingSpec {
    fn default() -> Self {
        LossHandlingSpec::ConfidenceWeighted
    }
}
