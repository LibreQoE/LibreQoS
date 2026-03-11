use crate::config::StormguardConfig;
use crate::config::WatchingSite;
use crate::site_state::analysis::{RetransmitState, RttState, SaturationLevel};
use crate::site_state::recommendation::{
    Recommendation, RecommendationAction, RecommendationDirection,
};
use crate::site_state::ring_buffer::RingBuffer;
use crate::site_state::stormguard_state::StormguardState;
use allocative::Allocative;
use std::time::Instant;
use tracing::{debug, info};

pub struct SiteState {
    pub config: WatchingSite,
    pub download_state: StormguardState,
    pub upload_state: StormguardState,

    // Queue Bandwidth
    pub queue_download_mbps: u64,
    pub queue_upload_mbps: u64,
    pub current_throughput: (f64, f64),
    /// Effective RTT used for DelayProbe/DelayProbeActive evaluation (may be blended).
    pub current_rtt_ms: Option<f64>,
    /// Most recent passive RTT sample (TCP-derived), if available.
    pub passive_rtt_ms: Option<f64>,
    /// Most recent active ping RTT sample, if available.
    pub active_ping_rtt_ms: Option<f64>,
    /// When set, represents a newly-arrived effective RTT sample (used for baseline learning).
    pub rtt_sample_for_baseline_ms: Option<f64>,
    pub rtt_baseline_ms: Option<f64>,
    pub(crate) last_passive_rtt_ms: Option<f64>,
    pub(crate) last_passive_rtt_at: Option<Instant>,
    pub(crate) passive_rtt_updated_this_tick: bool,
    pub(crate) last_action_download: Option<(RecommendationAction, Instant)>,
    pub(crate) last_action_upload: Option<(RecommendationAction, Instant)>,

    // Current Data Buffers
    pub throughput_down: RingBuffer,
    pub throughput_up: RingBuffer,
    pub retransmits_down: RingBuffer,
    pub retransmits_up: RingBuffer,
    pub round_trip_time: RingBuffer,

    // Moving Average Buffers
    pub throughput_down_moving_average: RingBuffer,
    pub throughput_up_moving_average: RingBuffer,
    pub retransmits_down_moving_average: RingBuffer,
    pub retransmits_up_moving_average: RingBuffer,
    pub round_trip_time_moving_average: RingBuffer,

    // Increase Ticker
    pub ticks_since_last_probe_download: u32,
    pub ticks_since_last_probe_upload: u32,
}

#[derive(Allocative)]
struct RecommendationParams {
    direction: RecommendationDirection,
    can_increase: bool,
    can_decrease: bool,
    saturation_max: SaturationLevel,
    saturation_current: SaturationLevel,
    retransmit_state: RetransmitState,
    rtt_state: RttState,
    /// Absolute retransmit fraction (0.10 = 10%)
    abs_retransmit: Option<f64>,
}

impl RecommendationParams {
    fn summary_string(&self) -> String {
        format!(
            "{},{:?},{:?},{},{},{},{},abs_retx={:?}",
            self.direction,
            self.can_increase,
            self.can_decrease,
            self.saturation_max,
            self.saturation_current,
            self.retransmit_state,
            self.rtt_state,
            self.abs_retransmit
        )
    }
}

impl SiteState {
    pub fn check_state(&mut self, config: &StormguardConfig) {
        self.update_rtt_baseline(config);

        self.check_state_direction(RecommendationDirection::Download);
        self.check_state_direction(RecommendationDirection::Upload);

        if !matches!(self.download_state, StormguardState::Warmup)
            || !matches!(self.upload_state, StormguardState::Warmup)
        {
            self.moving_averages_rtt();
        }
    }

    fn check_state_direction(&mut self, direction: RecommendationDirection) {
        let (state, throughput, retransmits, throughput_ma, retransmits_ma, direction_name) =
            match direction {
                RecommendationDirection::Download => (
                    &mut self.download_state,
                    &self.throughput_down,
                    &self.retransmits_down,
                    &mut self.throughput_down_moving_average,
                    &mut self.retransmits_down_moving_average,
                    "download",
                ),
                RecommendationDirection::Upload => (
                    &mut self.upload_state,
                    &self.throughput_up,
                    &self.retransmits_up,
                    &mut self.throughput_up_moving_average,
                    &mut self.retransmits_up_moving_average,
                    "upload",
                ),
            };

        match state {
            StormguardState::Warmup => {
                // Do we have enough data to consider ourselves functional?
                if throughput.count() > 10 && retransmits.count() > 10 {
                    info!(
                        "Site {} has completed {direction_name} warm-up.",
                        self.config.name
                    );
                    *state = StormguardState::Running;
                }
            }
            StormguardState::Running => {
                Self::push_moving_average(throughput, throughput_ma);
                Self::push_moving_average(retransmits, retransmits_ma);
            }
            StormguardState::Cooldown {
                start,
                duration_secs,
            } => {
                Self::push_moving_average(throughput, throughput_ma);
                Self::push_moving_average(retransmits, retransmits_ma);

                // Check if cooldown period is over
                let now = std::time::Instant::now();
                if now.duration_since(*start).as_secs_f32() > *duration_secs {
                    debug!(
                        "Site {} has completed {direction_name} cooldown.",
                        self.config.name
                    );
                    *state = StormguardState::Running;
                }
            }
        }
    }

    fn push_moving_average(source: &RingBuffer, target: &mut RingBuffer) {
        if let Some(value) = source.average() {
            target.add(value);
        }
    }

    fn update_rtt_baseline(&mut self, config: &StormguardConfig) {
        if !matches!(
            config.strategy,
            lqos_config::StormguardStrategy::DelayProbe
                | lqos_config::StormguardStrategy::DelayProbeActive
        ) {
            return;
        }
        let Some(rtt_ms) = self.rtt_sample_for_baseline_ms else {
            return;
        };

        self.rtt_baseline_ms = Some(match self.rtt_baseline_ms {
            None => rtt_ms,
            Some(baseline_ms) => {
                let alpha = if rtt_ms > baseline_ms {
                    config.baseline_alpha_up
                } else {
                    config.baseline_alpha_down
                }
                .clamp(0.0, 1.0) as f64;
                baseline_ms + alpha * (rtt_ms - baseline_ms)
            }
        });
    }

    pub(crate) fn record_passive_rtt_sample(&mut self, rtt_ms: f64) {
        self.last_passive_rtt_ms = Some(rtt_ms);
        self.last_passive_rtt_at = Some(Instant::now());
        self.passive_rtt_updated_this_tick = true;
    }

    pub(crate) fn clear_tick_rtt_state(&mut self) {
        self.current_rtt_ms = None;
        self.passive_rtt_ms = None;
        self.active_ping_rtt_ms = None;
        self.rtt_sample_for_baseline_ms = None;
        self.passive_rtt_updated_this_tick = false;
    }

    pub(crate) fn last_passive_rtt(&self) -> Option<(f64, Instant)> {
        match (self.last_passive_rtt_ms, self.last_passive_rtt_at) {
            (Some(ms), Some(at)) => Some((ms, at)),
            _ => None,
        }
    }

    pub(crate) fn passive_rtt_updated_this_tick(&self) -> bool {
        self.passive_rtt_updated_this_tick
    }

    fn moving_averages_rtt(&mut self) {
        Self::push_moving_average(&self.round_trip_time, &mut self.round_trip_time_moving_average);
    }

    fn recommendation_matrix(
        &mut self,
        recommendations: &mut Vec<(Recommendation, String)>,
        params: &RecommendationParams,
    ) {
        if !params.can_increase && !params.can_decrease {
            return; // No recommendations possible
        }

        let (rtt_weight, retransmit_weight, score_bias) = match params.saturation_current {
            SaturationLevel::High => (3.0, 1.0, 0.0),
            SaturationLevel::Medium => (2.0, 1.0, 0.0),
            SaturationLevel::Low => (1.5, 1.0, 0.0),
        };

        // Calculate the score based on the recommendation parameters
        let score_base = score_bias;

        let score_rtt = match &params.rtt_state {
            RttState::Rising { magnitude } => magnitude.abs() * rtt_weight, // punish
            RttState::Flat => 0.0,
            RttState::Falling { magnitude } => -magnitude.abs() * rtt_weight, // reward
        };

        let score_retransmit = match &params.retransmit_state {
            RetransmitState::RisingFast => 1.5 * retransmit_weight,
            RetransmitState::Rising => 1.0 * retransmit_weight,
            RetransmitState::Stable => 0.0, // No change
            RetransmitState::Falling => -1.0 * retransmit_weight,
            RetransmitState::FallingFast => -1.5 * retransmit_weight,
        };

        // Absolute retransmit penalty: if loss > 10%, push toward decrease even if stable.
        let high_loss_penalty = params
            .abs_retransmit
            .and_then(|p| if p >= 0.10 { Some(3.0) } else { None })
            .unwrap_or(0.0);

        // Tick Bias
        /*let tick_bias = match params.direction {
            RecommendationDirection::Download => self.ticks_since_last_probe_download as f32,
            RecommendationDirection::Upload => self.ticks_since_last_probe_upload as f32,
        };
        let score_tick = match params.saturation_current {
            SaturationLevel::High => -f32::min(2.0, tick_bias * 0.4), // Positive bias that grows with time
            SaturationLevel::Medium => 0.0,
            SaturationLevel::Low => -f32::min(5.0, tick_bias),
        };*/
        let score_tick = 0.0; // Removed for now

        let score_stability_bonus =
            if matches!(params.rtt_state, RttState::Flat | RttState::Falling { .. })
                && matches!(
                    params.retransmit_state,
                    RetransmitState::Stable
                        | RetransmitState::Falling
                        | RetransmitState::FallingFast
                )
                && params.saturation_current == SaturationLevel::Low
            {
                -1.5 // Stronger bonus for stable operation
            } else {
                0.0
            };

        let score = score_base
            + score_rtt
            + score_retransmit
            + score_stability_bonus
            + score_tick
            + high_loss_penalty;
        debug!("{} : {}", params.direction, params.summary_string());
        debug!(
            "Score {}: {score_base:.1}(base) + {score_rtt:1}(rtt) + {score_retransmit:.1}(retransmit) {score_stability_bonus:.2}(stable) + {score_tick:.1}(tick) + {high_loss_penalty:.1}(abs_retx) = {score:.1}",
            params.direction
        );

        // Determine the recommendation action
        let action = match score {
            score if score < -1.0 => Some(RecommendationAction::IncreaseFast), // Easier to increase
            score if score > 3.0 => Some(RecommendationAction::DecreaseFast),  // Harder to decrease
            score if score < 0.0 => Some(RecommendationAction::Increase), // Wider increase band
            score if score > 2.0 => Some(RecommendationAction::Decrease), // Narrower decrease band
            _ => None,
        };
        //println!("Score: {score}, recommendation: {:?}", action);

        if let Some(action) = action {
            match action {
                RecommendationAction::IncreaseFast | RecommendationAction::Increase => {
                    if params.can_increase {
                        recommendations.push((
                            Recommendation {
                                site: self.config.name.to_owned(),
                                action,
                                direction: params.direction,
                            },
                            params.summary_string(),
                        ));
                    }
                }
                RecommendationAction::DecreaseFast | RecommendationAction::Decrease => {
                    if params.can_decrease {
                        recommendations.push((
                            Recommendation {
                                site: self.config.name.to_owned(),
                                action,
                                direction: params.direction,
                            },
                            params.summary_string(),
                        ));
                    }
                }
            }
        }
    }

    fn recommendations_legacy_score_direction(
        &mut self,
        recommendations: &mut Vec<(Recommendation, String)>,
        direction: RecommendationDirection,
    ) {
        let (queue_mbps, min_mbps, max_mbps, throughput_mbps, retransmits_ma, retransmits) =
            match direction {
                RecommendationDirection::Download => (
                    self.queue_download_mbps,
                    self.config.min_download_mbps,
                    self.config.max_download_mbps,
                    self.current_throughput.0,
                    &self.retransmits_down_moving_average,
                    &self.retransmits_down,
                ),
                RecommendationDirection::Upload => (
                    self.queue_upload_mbps,
                    self.config.min_upload_mbps,
                    self.config.max_upload_mbps,
                    self.current_throughput.1,
                    &self.retransmits_up_moving_average,
                    &self.retransmits_up,
                ),
            };

        let saturation_max = SaturationLevel::from_throughput(throughput_mbps, max_mbps as f64);
        let saturation_current = SaturationLevel::from_throughput(throughput_mbps, queue_mbps as f64);
        let retransmit_state = RetransmitState::new(retransmits_ma, retransmits);
        let abs_retransmit = retransmits_ma.average();
        let rtt_state = RttState::new(&self.round_trip_time_moving_average, &self.round_trip_time);

        let params = RecommendationParams {
            direction,
            can_increase: queue_mbps < max_mbps,
            can_decrease: queue_mbps > min_mbps,
            saturation_max,
            saturation_current,
            retransmit_state,
            rtt_state,
            abs_retransmit,
        };

        self.recommendation_matrix(recommendations, &params);
    }

    fn recommendations_delay_probe_direction(
        &mut self,
        recommendations: &mut Vec<(Recommendation, String)>,
        config: &StormguardConfig,
        direction: RecommendationDirection,
    ) {
        let threshold_ms = config.delay_threshold_ms as f64;
        let threshold_ratio = config.delay_threshold_ratio as f64;
        let fast_threshold_ms = threshold_ms * 2.0;
        let fast_threshold_ratio = 1.0 + (threshold_ratio - 1.0) * 2.0;
        let good_threshold_ms = threshold_ms * 0.5;
        let good_threshold_ratio = 1.0 + (threshold_ratio - 1.0) * 0.5;
        let probe_interval_ticks = config.probe_interval_seconds.max(1.0).round() as u32;

        let (queue_mbps, min_mbps, max_mbps, throughput_mbps, retransmits_avg) = match direction {
            RecommendationDirection::Download => (
                self.queue_download_mbps,
                self.config.min_download_mbps,
                self.config.max_download_mbps,
                self.current_throughput.0,
                self.retransmits_down.average(),
            ),
            RecommendationDirection::Upload => (
                self.queue_upload_mbps,
                self.config.min_upload_mbps,
                self.config.max_upload_mbps,
                self.current_throughput.1,
                self.retransmits_up.average(),
            ),
        };

        let can_increase = queue_mbps < max_mbps;
        let can_decrease = queue_mbps > min_mbps;
        if !can_increase && !can_decrease {
            return;
        }

        let mut delay_ms: Option<f64> = None;
        let mut delay_ratio: Option<f64> = None;
        let mut bloat = false;
        let mut severe_bloat = false;

        let rtt_allowed = if matches!(
            config.strategy,
            lqos_config::StormguardStrategy::DelayProbeActive
        ) && self.active_ping_rtt_ms.is_some()
        {
            true
        } else {
            throughput_mbps >= config.min_throughput_mbps_for_rtt as f64
        };

        if rtt_allowed {
            if let (Some(rtt_ms), Some(baseline_ms)) = (self.current_rtt_ms, self.rtt_baseline_ms) {
                let baseline_ms = baseline_ms.max(1.0);
                let delay = (rtt_ms - baseline_ms).max(0.0);
                let ratio = rtt_ms / baseline_ms;
                delay_ms = Some(delay);
                delay_ratio = Some(ratio);

                bloat = delay >= threshold_ms || ratio >= threshold_ratio;
                severe_bloat = delay >= fast_threshold_ms || ratio >= fast_threshold_ratio;
            }
        }

        let high_loss = retransmits_avg.is_some_and(|p| p >= 0.10);
        let moderate_loss = retransmits_avg.is_some_and(|p| p >= 0.05);

        let action = if can_decrease && (severe_bloat || high_loss) {
            Some(RecommendationAction::DecreaseFast)
        } else if can_decrease && (bloat || moderate_loss) {
            Some(RecommendationAction::Decrease)
        } else if can_increase {
            let Some(delay_ms) = delay_ms else {
                return;
            };
            let Some(delay_ratio) = delay_ratio else {
                return;
            };
            let good_delay =
                delay_ms <= good_threshold_ms && delay_ratio <= good_threshold_ratio;
            let load_ratio = if queue_mbps > 0 {
                throughput_mbps / queue_mbps as f64
            } else {
                0.0
            };
            let ticks_since_last_probe = match direction {
                RecommendationDirection::Download => self.ticks_since_last_probe_download,
                RecommendationDirection::Upload => self.ticks_since_last_probe_upload,
            };
            if good_delay && load_ratio >= 0.80 && ticks_since_last_probe >= probe_interval_ticks {
                Some(RecommendationAction::Increase)
            } else {
                None
            }
        } else {
            None
        };

        let Some(action) = action else {
            return;
        };

        let summary = format!(
            "{direction},{action:?},queue={queue_mbps},tp={throughput_mbps:.3},retx={retransmits_avg:?},rtt={:?},baseline={:?},delay_ms={:?},delay_ratio={:?},bloat={bloat},severe={severe_bloat}",
            self.current_rtt_ms,
            self.rtt_baseline_ms,
            delay_ms,
            delay_ratio,
        );

        recommendations.push((
            Recommendation {
                site: self.config.name.to_owned(),
                action,
                direction,
            },
            summary,
        ));
    }

    pub fn recommendations(
        &mut self,
        recommendations: &mut Vec<(Recommendation, String)>,
        config: &StormguardConfig,
    ) {
        match config.strategy {
            lqos_config::StormguardStrategy::DelayProbe
            | lqos_config::StormguardStrategy::DelayProbeActive => {
                self.ticks_since_last_probe_download =
                    self.ticks_since_last_probe_download.saturating_add(1);
                self.ticks_since_last_probe_upload =
                    self.ticks_since_last_probe_upload.saturating_add(1);

                if !matches!(self.download_state, StormguardState::Cooldown { .. }) {
                    self.recommendations_delay_probe_direction(
                        recommendations,
                        config,
                        RecommendationDirection::Download,
                    );
                }
                if !matches!(self.upload_state, StormguardState::Cooldown { .. }) {
                    self.recommendations_delay_probe_direction(
                        recommendations,
                        config,
                        RecommendationDirection::Upload,
                    );
                }
            }
            lqos_config::StormguardStrategy::LegacyScore => {
                if self.download_state == StormguardState::Running {
                    self.recommendations_legacy_score_direction(
                        recommendations,
                        RecommendationDirection::Download,
                    );
                    self.ticks_since_last_probe_download =
                        self.ticks_since_last_probe_download.saturating_add(1);
                }
                if self.upload_state == StormguardState::Running {
                    self.recommendations_legacy_score_direction(
                        recommendations,
                        RecommendationDirection::Upload,
                    );
                    self.ticks_since_last_probe_upload =
                        self.ticks_since_last_probe_upload.saturating_add(1);
                }
            }
        }
    }
}
