//! QoO (Quality of Outcome) scoring utilities for LibreQoS.
//!
//! This module implements the QoO scoring math described in
//! draft-ietf-ippm-qoo-06 §7.1 (Latency Component, Packet Loss Component, Overall QoO). It also
//! provides a LibreQoS-oriented combination mode that includes throughput in the final `min()`.
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
    HigherIsBetter,
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
    pub low: f64,
    pub high: f64,
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
/// - Draft-defined min(latency, loss), or
/// - LibreQoS-oriented min(latency, loss, download_throughput, upload_throughput).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CombineMode {
    IetfLatencyAndLoss,
    LibreQosLatencyLossThroughput,
}

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
    ThresholdOffsetMs { ms: f64 },

    /// Score “excess RTT” above a baseline.
    ExcessOverBaseline { baseline: Baseline },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Baseline {
    FixedMs { ms: f64 },
    Percentile { percentile: u8 },
}

/// Profile for computing QoO.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QooProfile {
    pub name: String,

    /// Whether to use a windowed bucket or lifetime/total bucket.
    pub rtt_scope: RttBucket,

    /// Throughput thresholds in Mbps (higher is better).
    pub download_mbps: LowHigh,
    pub upload_mbps: LowHigh,

    /// One or more latency percentiles (lower is better).
    pub latency: Vec<LatencyReq>,

    /// Loss thresholds as a FRACTION (0.01 = 1%), lower is better.
    pub loss_fraction: LowHigh,

    pub combine: CombineMode,
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
        loss_fraction: f64, // 0..1
    },

    /// Loss fraction is inferred from TCP retransmits (low confidence by nature).
    TcpRetransmitProxy {
        retransmit_fraction: f64, // 0..1
        confidence: f64,          // 0..1
    },
}

impl LossMeasurement {
    pub fn loss_fraction(&self) -> f64 {
        match *self {
            LossMeasurement::Exact { loss_fraction } => loss_fraction.clamp(0.0, 1.0),
            LossMeasurement::TcpRetransmitProxy {
                retransmit_fraction,
                ..
            } => retransmit_fraction.clamp(0.0, 1.0),
        }
    }

    pub fn confidence(&self) -> f64 {
        match *self {
            LossMeasurement::Exact { .. } => 1.0,
            LossMeasurement::TcpRetransmitProxy { confidence, .. } => confidence.clamp(0.0, 1.0),
        }
    }

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

    /// Typically: shaper rate / plan rate in Mbps.
    pub download_mbps: f64,
    pub upload_mbps: f64,

    /// Loss (or proxy) measurement.
    pub loss: Option<LossMeasurement>,
}

/// Component breakdown (useful for GUI tooltips).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QooComponents {
    pub latency_download: Option<f64>,
    pub latency_upload: Option<f64>,
    pub latency_worst: Option<f64>,

    pub loss_strict: Option<f64>,
    pub loss_effective: Option<f64>,
    pub loss_confidence: Option<f64>,

    pub throughput_download: Option<f64>,
    pub throughput_upload: Option<f64>,
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
    pub latency_upload_scored_ms: Vec<(u8, f64)>,

    /// Baseline/offset used for normalization (if any).
    pub latency_baseline_download_ms: Option<f64>,
    pub latency_baseline_upload_ms: Option<f64>,

    pub download_mbps: Option<f64>,
    pub upload_mbps: Option<f64>,

    /// Packet loss fraction (0..1) or proxy.
    pub loss_fraction: Option<f64>,
}

/// Result of a QoO computation.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QooResult {
    /// Final QoO score (0..100). None if insufficient data.
    pub overall: Option<f64>,

    pub components: QooComponents,
    pub measured: QooMeasured,
}

/// Compute QoO for a given profile and dataset.
///
/// - Latency scoring: compute a score for each latency percentile requirement and take the minimum.
/// - Download vs upload latency: computed separately; `latency_worst` is `min(dl, ul)`.
/// - Loss scoring: a single score based on overall loss fraction (or proxy).
/// - Overall: min of component scores as specified by `profile.combine`.
pub fn compute_qoo(profile: &QooProfile, input: &QooInput<'_>) -> QooResult {
    let mut out = QooResult::default();

    // Throughput components.
    out.measured.download_mbps = Some(input.download_mbps);
    out.measured.upload_mbps = Some(input.upload_mbps);

    out.components.throughput_download = Some(profile.download_mbps.score(input.download_mbps));
    out.components.throughput_upload = Some(profile.upload_mbps.score(input.upload_mbps));

    // Latency components from RTT histograms.
    let dl = latency_for_direction(
        profile,
        input.rtt,
        FlowbeeEffectiveDirection::Download,
        profile.rtt_scope,
    );
    let ul = latency_for_direction(
        profile,
        input.rtt,
        FlowbeeEffectiveDirection::Upload,
        profile.rtt_scope,
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
    out.overall = match profile.combine {
        CombineMode::IetfLatencyAndLoss => match (out.components.latency_worst, out.components.loss_effective) {
            (Some(l), Some(p)) => Some(l.min(p)),
            _ => None,
        },
        CombineMode::LibreQosLatencyLossThroughput => {
            let latency = out.components.latency_worst;
            let loss = out.components.loss_effective;

            if let (Some(latency), Some(loss)) = (latency, loss) {
                let td = out.components.throughput_download.unwrap_or(100.0);
                let tu = out.components.throughput_upload.unwrap_or(100.0);

                Some(latency.min(loss).min(td).min(tu))
            } else {
                None
            }
        }
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

pub const QOQ_UNKNOWN: u8 = 255;

/// QoQ scores for download/upload across both histogram scopes (Total vs Current).
///
/// Values are 0..100, with `QOQ_UNKNOWN` meaning "insufficient data".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Allocative)]
pub struct QoqScores {
    pub download_total: u8,
    pub upload_total: u8,
    pub download_current: u8,
    pub upload_current: u8,
}

impl Default for QoqScores {
    fn default() -> Self {
        Self {
            download_total: QOQ_UNKNOWN,
            upload_total: QOQ_UNKNOWN,
            download_current: QOQ_UNKNOWN,
            upload_current: QOQ_UNKNOWN,
        }
    }
}

impl QoqScores {
    #[inline]
    pub fn download_total_f32(self) -> Option<f32> {
        (self.download_total != QOQ_UNKNOWN).then(|| self.download_total as f32)
    }

    #[inline]
    pub fn upload_total_f32(self) -> Option<f32> {
        (self.upload_total != QOQ_UNKNOWN).then(|| self.upload_total as f32)
    }

    #[inline]
    pub fn download_current_f32(self) -> Option<f32> {
        (self.download_current != QOQ_UNKNOWN).then(|| self.download_current as f32)
    }

    #[inline]
    pub fn upload_current_f32(self) -> Option<f32> {
        (self.upload_current != QOQ_UNKNOWN).then(|| self.upload_current as f32)
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

fn combine_directional(
    profile: &QooProfile,
    latency: Option<f64>,
    loss: Option<f64>,
    throughput: f64,
) -> Option<f64> {
    match profile.combine {
        CombineMode::IetfLatencyAndLoss => match (latency, loss) {
            (Some(l), Some(p)) => Some(l.min(p)),
            _ => None,
        },
        CombineMode::LibreQosLatencyLossThroughput => match (latency, loss) {
            (Some(l), Some(p)) => Some(l.min(p).min(throughput)),
            _ => None,
        },
    }
}

/// Compute QoQ scores (0..100) for download/upload directions using both RTT histogram scopes:
/// `RttBucket::Total` and `RttBucket::Current`.
///
/// The only difference between Total vs Current outputs is the RTT bucket scope used for latency.
pub fn compute_qoq_scores(
    profile: &QooProfile,
    rtt: &RttBuffer,
    download_mbps: f64,
    upload_mbps: f64,
    loss_download: Option<LossMeasurement>,
    loss_upload: Option<LossMeasurement>,
) -> QoqScores {
    let throughput_download = profile.download_mbps.score(download_mbps);
    let throughput_upload = profile.upload_mbps.score(upload_mbps);

    let loss_download = loss_download.map(|l| loss_effective(profile, l));
    let loss_upload = loss_upload.map(|l| loss_effective(profile, l));

    let dl_current = latency_for_direction(profile, rtt, FlowbeeEffectiveDirection::Download, RttBucket::Current).score;
    let ul_current = latency_for_direction(profile, rtt, FlowbeeEffectiveDirection::Upload, RttBucket::Current).score;
    let dl_total = latency_for_direction(profile, rtt, FlowbeeEffectiveDirection::Download, RttBucket::Total).score;
    let ul_total = latency_for_direction(profile, rtt, FlowbeeEffectiveDirection::Upload, RttBucket::Total).score;

    QoqScores {
        download_total: score_to_u8(combine_directional(profile, dl_total, loss_download, throughput_download)),
        upload_total: score_to_u8(combine_directional(profile, ul_total, loss_upload, throughput_upload)),
        download_current: score_to_u8(combine_directional(profile, dl_current, loss_download, throughput_download)),
        upload_current: score_to_u8(combine_directional(profile, ul_current, loss_upload, throughput_upload)),
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
