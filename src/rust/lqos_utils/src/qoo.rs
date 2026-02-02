//! QoO (Quality of Outcome) scoring utilities for LibreQoS.
//!
//! This module implements the QoO scoring math described in
//! draft-ietf-ippm-qoo-06 §7.1 (Latency Component, Packet Loss Component, Overall QoO). It also
//!
//! Key design points for LibreQoS:
//! - Profiles are configured with **two thresholds** per metric: a "good/target" threshold and a
//!   "bad/minimum" threshold, and measured values are linearly interpolated into [0, 100].
//! - Latency is typically computed from a percentile (e.g. p95) derived from `RttBuffer`.
//! - Packet loss is often not directly available for a passive bridge; a TCP retransmit fraction can
//!   be used as a proxy, with an explicit confidence value.

mod qoo_profile;

pub use qoo_profile::{ProfileIoError, QooProfileSpec, QooProfilesFile};

use allocative::Allocative;
use serde::{Deserialize, Serialize};
use crate::rtt::{FlowbeeEffectiveDirection, RttBucket, RttBuffer};
use smallvec::SmallVec;

#[inline]
fn clamp_0_100(x: f64) -> f64 {
    x.max(0.0).min(100.0)
}

#[inline]
fn nanos_to_ms(ns: u64) -> f64 {
    ns as f64 / 1_000_000.0
}

/// Whether larger values are better or worse for a metric.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Better {
    /// Larger measured values should receive higher scores.
    HigherIsBetter,
    /// Smaller measured values should receive higher scores.
    LowerIsBetter,
}

/// Two-threshold range for interpolation into [0, 100].
///
/// Interpretation:
/// - For LowerIsBetter metrics (latency/loss):
///   - `high` = "good/target" threshold (ROP in the QoO draft)
///   - `low`  = "bad/minimum" threshold (CPUP in the QoO draft)
/// - For HigherIsBetter metrics (throughput):
///   - `low`  = minimum acceptable
///   - `high` = target
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct LowHigh {
    /// The "low" threshold:
    /// - For `LowerIsBetter` metrics, values >= `low` are treated as "bad/unacceptable".
    /// - For `HigherIsBetter` metrics, values <= `low` are treated as "bad/unacceptable".
    pub low: f64,
    /// The "high" threshold:
    /// - For `LowerIsBetter` metrics, values <= `high` are treated as "good/target".
    /// - For `HigherIsBetter` metrics, values >= `high` are treated as "good/target".
    pub high: f64,
    /// Whether higher or lower values are better for this metric.
    pub better: Better,
}

impl LowHigh {
    /// Throughput-like metrics: higher is better (`low`=minimum, `high`=target).
    pub fn higher_is_better(low: f64, high: f64) -> Self {
        Self {
            low,
            high,
            better: Better::HigherIsBetter,
        }
    }

    /// Latency/loss-like metrics: lower is better (`high`=target/ROP, `low`=bad/CPUP).
    pub fn lower_is_better(low: f64, high: f64) -> Self {
        Self {
            low,
            high,
            better: Better::LowerIsBetter,
        }
    }

    /// Score a measured value into [0, 100] by linear interpolation + clamp.
    ///
    /// For lower-is-better, this matches the draft’s form:
    ///   score = clamp( (1 - (ML - ROP)/(CPUP - ROP)) * 100, 0, 100 )
    ///
    /// For higher-is-better, it uses the symmetric form.
    pub fn score(&self, measured: f64) -> f64 {
        if !measured.is_finite() {
            return 0.0;
        }

        match self.better {
            Better::LowerIsBetter => {
                // "high" is good/target (ROP), "low" is bad/unacceptable (CPUP)
                let good = self.high;
                let bad = self.low;

                // Degenerate thresholds -> step function.
                let denom = bad - good;
                if denom <= 0.0 {
                    return if measured <= good { 100.0 } else { 0.0 };
                }

                if measured <= good {
                    100.0
                } else if measured >= bad {
                    0.0
                } else {
                    clamp_0_100(100.0 * (bad - measured) / denom)
                }
            }
            Better::HigherIsBetter => {
                // "high" is good/target, "low" is minimum acceptable
                let good = self.high;
                let bad = self.low;

                let denom = good - bad;
                if denom <= 0.0 {
                    return if measured >= good { 100.0 } else { 0.0 };
                }

                if measured >= good {
                    100.0
                } else if measured <= bad {
                    0.0
                } else {
                    clamp_0_100(100.0 * (measured - bad) / denom)
                }
            }
        }
    }
}

/// Latency requirement at a specific percentile (e.g., p95).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LatencyReq {
    /// Percentile to evaluate (e.g. 95 for p95).
    pub percentile: u8,
    /// RTT in milliseconds; lower is better.
    pub rtt_ms: LowHigh,
}

/// How to incorporate loss proxy confidence (if desired).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LossHandling {
    /// Use loss score directly in the final `min()`.
    Strict,

    /// Blend loss score toward 100 based on confidence:
    ///   effective = 100 - confidence * (100 - strict)
    ///
    /// This can reduce UI “overconfidence” when loss is inferred (e.g. TCP retransmits).
    ConfidenceWeighted,
}

/// Whether to compute overall QoO as:
/// - Draft-defined min(latency, loss).
///
/// Latency bias/normalization options.
///
/// Use cases:
/// - Many deployments have unavoidable baseline RTT (geography/backhaul). If you want to measure
///   "how much extra latency is being added" (bufferbloat), subtract a baseline.
/// - If you want an "absolute outcome score", use `None`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum LatencyNormalization {
    /// Score absolute RTT directly against thresholds.
    None,

    /// Subtract a fixed offset before scoring (equivalent to shifting thresholds upward).
    ///
    /// The offset is applied as: `scored_ms = max(raw_ms - ms, 0)`.
    ThresholdOffsetMs {
        /// Offset in milliseconds.
        ms: f64,
    },

    /// Score “excess RTT” above a baseline.
    ExcessOverBaseline {
        /// Baseline definition (fixed ms or another RTT percentile).
        baseline: Baseline,
    },
}

/// Baseline definition for `LatencyNormalization::ExcessOverBaseline`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Baseline {
    /// Use a fixed baseline in milliseconds.
    FixedMs {
        /// Baseline in milliseconds.
        ms: f64,
    },
    /// Use another RTT percentile (e.g. p50) from the same RTT histogram as baseline.
    Percentile {
        /// Percentile to use as baseline (e.g. 50 for p50).
        percentile: u8,
    },
}

/// Profile for computing QoO.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QooProfile {
    /// Human readable profile name (for UI display).
    pub name: String,

    /// One or more latency percentiles (lower is better).
    pub latency: Vec<LatencyReq>,

    /// Loss thresholds as a FRACTION (0.01 = 1%), lower is better.
    pub loss_fraction: LowHigh,

    /// How to incorporate confidence when using a loss proxy.
    pub loss_handling: LossHandling,

    /// Optional baseline/bias handling for latency.
    #[serde(default)]
    pub latency_normalization: LatencyNormalization,
}

impl Default for LatencyNormalization {
    fn default() -> Self {
        LatencyNormalization::None
    }
}

/// Loss measurement input.
///
/// For a passive bridge, `TcpRetransmitProxy` is typically the only option.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum LossMeasurement {
    /// Loss fraction is known/authoritative (confidence=1).
    Exact {
        /// Loss fraction in the range 0..1.
        loss_fraction: f64,
    },

    /// Loss fraction is inferred from TCP retransmits (low confidence by nature).
    TcpRetransmitProxy {
        /// Retransmit fraction in the range 0..1, used as a packet-loss proxy.
        retransmit_fraction: f64,
        /// Confidence in the retransmit proxy in the range 0..1.
        confidence: f64,
    },
}

impl LossMeasurement {
    /// Return the (possibly-proxied) loss fraction in the range 0..1.
    pub fn loss_fraction(&self) -> f64 {
        match *self {
            LossMeasurement::Exact { loss_fraction } => loss_fraction.clamp(0.0, 1.0),
            LossMeasurement::TcpRetransmitProxy {
                retransmit_fraction,
                ..
            } => retransmit_fraction.clamp(0.0, 1.0),
        }
    }

    /// Return the confidence of the loss measurement in the range 0..1.
    pub fn confidence(&self) -> f64 {
        match *self {
            LossMeasurement::Exact { .. } => 1.0,
            LossMeasurement::TcpRetransmitProxy { confidence, .. } => confidence.clamp(0.0, 1.0),
        }
    }

    /// Build a retransmit-based loss proxy from a percent value (0..100) and confidence (0..1).
    pub fn from_tcp_retransmit_percent(retransmit_percent: f64, confidence: f64) -> Self {
        LossMeasurement::TcpRetransmitProxy {
            retransmit_fraction: (retransmit_percent / 100.0).clamp(0.0, 1.0),
            confidence: confidence.clamp(0.0, 1.0),
        }
    }
}

/// Input dataset for a single QoO computation.
#[derive(Clone, Debug)]
pub struct QooInput<'a> {
    /// RTT stats for the entity you’re scoring (subscriber / site / etc).
    pub rtt: &'a RttBuffer,

    /// Loss (or proxy) measurement.
    pub loss: Option<LossMeasurement>,
}

/// Component breakdown (useful for GUI tooltips).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QooComponents {
    /// Latency score for download direction (0..100).
    pub latency_download: Option<f64>,
    /// Latency score for upload direction (0..100).
    pub latency_upload: Option<f64>,
    /// Worst-of-directions latency score (min of download/upload).
    pub latency_worst: Option<f64>,

    /// Strict loss score (confidence ignored) in the range 0..100.
    pub loss_strict: Option<f64>,
    /// Effective loss score after applying the profile’s `loss_handling` in the range 0..100.
    pub loss_effective: Option<f64>,
    /// Confidence used to compute `loss_effective` in the range 0..1.
    pub loss_confidence: Option<f64>,
}

/// Measured/raw values (useful for debugging and presenting in the GUI).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QooMeasured {
    /// (percentile, ms) for download direction (raw RTT)
    pub latency_download_ms: Vec<(u8, f64)>,
    /// (percentile, ms) for upload direction (raw RTT)
    pub latency_upload_ms: Vec<(u8, f64)>,

    /// (percentile, ms) after applying latency normalization (the value actually scored)
    pub latency_download_scored_ms: Vec<(u8, f64)>,
    /// (percentile, ms) after applying latency normalization (the value actually scored)
    pub latency_upload_scored_ms: Vec<(u8, f64)>,

    /// Baseline/offset used for normalization (if any).
    pub latency_baseline_download_ms: Option<f64>,
    /// Baseline/offset used for normalization (if any).
    pub latency_baseline_upload_ms: Option<f64>,

    /// Packet loss fraction (0..1) or proxy.
    pub loss_fraction: Option<f64>,
}

/// Result of a QoO computation.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QooResult {
    /// Final QoO score (0..100). None if insufficient data.
    pub overall: Option<f64>,

    /// Component scores used to compute `overall`.
    pub components: QooComponents,
    /// Measured values and intermediate numbers used for scoring.
    pub measured: QooMeasured,
}

/// Compute QoO for a given profile and dataset.
///
/// - Latency scoring: compute a score for each latency percentile requirement and take the minimum.
/// - Download vs upload latency: computed separately; `latency_worst` is `min(dl, ul)`.
/// - Loss scoring: a single score based on overall loss fraction (or proxy).
/// - Overall: min(latency_worst, loss_effective).
pub fn compute_qoo(profile: &QooProfile, input: &QooInput<'_>) -> QooResult {
    let mut out = QooResult::default();

    // Latency components from RTT histograms.
    let dl = latency_for_direction(
        profile,
        input.rtt,
        FlowbeeEffectiveDirection::Download,
        RttBucket::Total,
    );
    let ul = latency_for_direction(
        profile,
        input.rtt,
        FlowbeeEffectiveDirection::Upload,
        RttBucket::Total,
    );

    out.components.latency_download = dl.score;
    out.components.latency_upload = ul.score;

    out.measured.latency_download_ms = dl.raw_ms.into_iter().collect();
    out.measured.latency_upload_ms = ul.raw_ms.into_iter().collect();

    out.measured.latency_download_scored_ms = dl.scored_ms.into_iter().collect();
    out.measured.latency_upload_scored_ms = ul.scored_ms.into_iter().collect();

    out.measured.latency_baseline_download_ms = dl.baseline_ms;
    out.measured.latency_baseline_upload_ms = ul.baseline_ms;

    out.components.latency_worst = match (dl.score, ul.score) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    };

    // Loss component.
    if let Some(loss) = input.loss {
        let loss_fraction = loss.loss_fraction();
        let confidence = loss.confidence();
        out.measured.loss_fraction = Some(loss_fraction);

        let strict = profile.loss_fraction.score(loss_fraction);
        let effective = match profile.loss_handling {
            LossHandling::Strict => strict,
            LossHandling::ConfidenceWeighted => 100.0 - confidence * (100.0 - strict),
        };

        out.components.loss_strict = Some(strict);
        out.components.loss_effective = Some(effective);
        out.components.loss_confidence = Some(confidence);
    }

    // Combine into overall.
    out.overall = match (out.components.latency_worst, out.components.loss_effective) {
        (Some(l), Some(p)) => Some(l.min(p)),
        _ => None,
    };

    out
}

#[derive(Clone, Debug, Default)]
struct LatencyDirectionResult {
    score: Option<f64>,
    raw_ms: SmallVec<[(u8, f64); 3]>,
    scored_ms: SmallVec<[(u8, f64); 3]>,
    baseline_ms: Option<f64>,
}

/// Compute latency score for one direction.
fn latency_for_direction(
    profile: &QooProfile,
    rtt: &RttBuffer,
    direction: FlowbeeEffectiveDirection,
    scope: RttBucket,
) -> LatencyDirectionResult {
    let mut raw_ms: SmallVec<[(u8, f64); 3]> = SmallVec::with_capacity(profile.latency.len());
    let mut scored_ms: SmallVec<[(u8, f64); 3]> = SmallVec::with_capacity(profile.latency.len());
    let mut scores: SmallVec<[f64; 3]> = SmallVec::with_capacity(profile.latency.len());

    // Determine baseline/offset for this direction if applicable.
    let baseline_ms: Option<f64> = match profile.latency_normalization {
        LatencyNormalization::None => None,
        LatencyNormalization::ThresholdOffsetMs { ms } => Some(ms.max(0.0)),
        LatencyNormalization::ExcessOverBaseline { baseline } => match baseline {
            Baseline::FixedMs { ms } => Some(ms.max(0.0)),
            Baseline::Percentile { percentile } => {
                let Some(b) = rtt.percentile(scope, direction, percentile) else {
                    return LatencyDirectionResult {
                        score: None,
                        raw_ms,
                        scored_ms,
                        baseline_ms: None,
                    };
                };
                Some(nanos_to_ms(b.as_nanos()).max(0.0))
            }
        },
    };

    for req in &profile.latency {
        let Some(rtt_p) = rtt.percentile(scope, direction, req.percentile) else {
            return LatencyDirectionResult {
                score: None,
                raw_ms,
                scored_ms,
                baseline_ms,
            };
        };

        let raw = nanos_to_ms(rtt_p.as_nanos());
        raw_ms.push((req.percentile, raw));

        let adjusted = match (profile.latency_normalization, baseline_ms) {
            (LatencyNormalization::None, _) => raw,
            (_, Some(b)) => (raw - b).max(0.0),
            // Should not happen, but be safe.
            (_, None) => raw,
        };

        scored_ms.push((req.percentile, adjusted));
        scores.push(req.rtt_ms.score(adjusted));
    }

    let score = scores.into_iter().reduce(f64::min);
    LatencyDirectionResult {
        score,
        raw_ms,
        scored_ms,
        baseline_ms,
    }
}

/// Sentinel value meaning "insufficient data" for QoO/QoQ scores.
pub const QOQ_UNKNOWN: u8 = 255;

/// QoO/QoQ scores for download/upload.
///
/// Values are 0..100, with `QOQ_UNKNOWN` meaning "insufficient data".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Allocative)]
pub struct QoqScores {
    /// Download-direction QoO/QoQ score (0..100), or `QOQ_UNKNOWN` if insufficient data.
    pub download_total: u8,
    /// Upload-direction QoO/QoQ score (0..100), or `QOQ_UNKNOWN` if insufficient data.
    pub upload_total: u8,
}

impl Default for QoqScores {
    fn default() -> Self {
        Self {
            download_total: QOQ_UNKNOWN,
            upload_total: QOQ_UNKNOWN,
        }
    }
}

impl QoqScores {
    /// Return the download score as `Some(f32)` if present, otherwise `None`.
    #[inline]
    pub fn download_total_f32(self) -> Option<f32> {
        (self.download_total != QOQ_UNKNOWN).then(|| self.download_total as f32)
    }

    /// Return the upload score as `Some(f32)` if present, otherwise `None`.
    #[inline]
    pub fn upload_total_f32(self) -> Option<f32> {
        (self.upload_total != QOQ_UNKNOWN).then(|| self.upload_total as f32)
    }
}

fn score_to_u8(score: Option<f64>) -> u8 {
    let Some(score) = score else { return QOQ_UNKNOWN };
    if !score.is_finite() {
        return QOQ_UNKNOWN;
    }
    score.clamp(0.0, 100.0).round() as u8
}

fn loss_effective(profile: &QooProfile, loss: LossMeasurement) -> f64 {
    let loss_fraction = loss.loss_fraction();
    let confidence = loss.confidence();
    let strict = profile.loss_fraction.score(loss_fraction);
    match profile.loss_handling {
        LossHandling::Strict => strict,
        LossHandling::ConfidenceWeighted => 100.0 - confidence * (100.0 - strict),
    }
}

fn combine_directional(latency: Option<f64>, loss: Option<f64>) -> Option<f64> {
    match (latency, loss) {
        (Some(l), Some(p)) => Some(l.min(p)),
        _ => None,
    }
}

const MIN_RTT_SAMPLES_FOR_QOO: u32 = 5;

/// Compute QoO/QoQ scores (0..100) for download/upload directions using the RTT histogram
/// `RttBucket::Total`.
pub fn compute_qoq_scores(
    profile: &QooProfile,
    rtt: &RttBuffer,
    loss_download: Option<LossMeasurement>,
    loss_upload: Option<LossMeasurement>,
) -> QoqScores {
    let loss_download = loss_download.map(|l| loss_effective(profile, l));
    let loss_upload = loss_upload.map(|l| loss_effective(profile, l));

    let dl_total = (rtt.sample_count(RttBucket::Total, FlowbeeEffectiveDirection::Download)
        >= MIN_RTT_SAMPLES_FOR_QOO)
        .then(|| {
            latency_for_direction(
                profile,
                rtt,
                FlowbeeEffectiveDirection::Download,
                RttBucket::Total,
            )
            .score
        })
        .flatten();
    let ul_total = (rtt.sample_count(RttBucket::Total, FlowbeeEffectiveDirection::Upload)
        >= MIN_RTT_SAMPLES_FOR_QOO)
        .then(|| {
            latency_for_direction(profile, rtt, FlowbeeEffectiveDirection::Upload, RttBucket::Total)
                .score
        })
        .flatten();

    QoqScores {
        download_total: score_to_u8(combine_directional(dl_total, loss_download)),
        upload_total: score_to_u8(combine_directional(ul_total, loss_upload)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lower_is_better_matches_expected_interpolation() {
        // Example-ish: good=200ms, bad=500ms.
        let rng = LowHigh::lower_is_better(500.0, 200.0);
        assert_eq!(rng.score(200.0), 100.0);
        assert_eq!(rng.score(500.0), 0.0);
        // midpoint: 350ms -> 50
        let s = rng.score(350.0);
        assert!((s - 50.0).abs() < 1e-9);
    }

    #[test]
    fn higher_is_better_interpolation() {
        // low=10, high=50
        let rng = LowHigh::higher_is_better(10.0, 50.0);
        assert_eq!(rng.score(50.0), 100.0);
        assert_eq!(rng.score(10.0), 0.0);
        // midpoint 30 -> 50
        let s = rng.score(30.0);
        assert!((s - 50.0).abs() < 1e-9);
    }

    #[test]
    fn confidence_weighted_loss_behaves() {
        let loss_rng = LowHigh::lower_is_better(0.05, 0.01);
        let strict = loss_rng.score(0.03); // between 1% and 5%
        let eff0 = 100.0 - 0.0 * (100.0 - strict);
        let eff1 = 100.0 - 1.0 * (100.0 - strict);
        assert_eq!(eff0, 100.0);
        assert!((eff1 - strict).abs() < 1e-9);
    }
}
