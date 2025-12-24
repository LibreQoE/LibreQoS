use crate::config::WatchingSite;
use crate::site_state::analysis::{RetransmitState, RttState, SaturationLevel};
use crate::site_state::recommendation::{
    Recommendation, RecommendationAction, RecommendationDirection,
};
use crate::site_state::ring_buffer::RingBuffer;
use crate::site_state::stormguard_state::StormguardState;
use allocative::Allocative;
use tracing::{debug, info};

pub struct SiteState {
    pub config: WatchingSite,
    pub download_state: StormguardState,
    pub upload_state: StormguardState,

    // Queue Bandwidth
    pub queue_download_mbps: u64,
    pub queue_upload_mbps: u64,
    pub current_throughput: (f64, f64),

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
    pub fn check_state(&mut self) {
        match self.download_state {
            StormguardState::Warmup => {
                // Do we have enough data to consider ourselves functional?
                let throughput_down_count = self.throughput_down.count();
                let retransmits_down_count = self.retransmits_down.count();
                if throughput_down_count > 10 && retransmits_down_count > 10 {
                    info!("Site {} has completed download warm-up.", self.config.name);
                    self.download_state = StormguardState::Running;
                }
            }
            StormguardState::Running => {
                self.moving_averages_down();
            }
            StormguardState::Cooldown {
                start,
                duration_secs,
            } => {
                self.moving_averages_down();

                // Check if cooldown period is over
                let now = std::time::Instant::now();
                if now.duration_since(start).as_secs_f32() > duration_secs {
                    debug!("Site {} has completed download cooldown.", self.config.name);
                    self.download_state = StormguardState::Running;
                    return;
                }
            }
        }

        match self.upload_state {
            StormguardState::Warmup => {
                // Do we have enough data to consider ourselves functional?
                let throughput_up_count = self.throughput_up.count();
                let retransmits_up_count = self.retransmits_up.count();
                if throughput_up_count > 10 && retransmits_up_count > 10 {
                    info!("Site {} has completed upload warm-up.", self.config.name);
                    self.upload_state = StormguardState::Running;
                }
            }
            StormguardState::Running => {
                self.moving_averages_up();
            }
            StormguardState::Cooldown {
                start,
                duration_secs,
            } => {
                self.moving_averages_up();

                // Check if cooldown period is over
                let now = std::time::Instant::now();
                if now.duration_since(start).as_secs_f32() > duration_secs {
                    debug!("Site {} has completed cooldown.", self.config.name);
                    self.upload_state = StormguardState::Running;
                }
            }
        }
    }

    pub fn moving_averages_down(&mut self) {
        let throughput_down = self.throughput_down.average();
        let retransmits_down = self.retransmits_down.average();
        let round_trip_time = self.round_trip_time.average();

        if let Some(throughput_down) = throughput_down {
            self.throughput_down_moving_average.add(throughput_down);
        }
        if let Some(retransmits_down) = retransmits_down {
            self.retransmits_down_moving_average.add(retransmits_down);
        }
        if let Some(round_trip_time) = round_trip_time {
            self.round_trip_time_moving_average.add(round_trip_time);
        }
    }

    pub fn moving_averages_up(&mut self) {
        let throughput_up = self.throughput_up.average();
        let retransmits_up = self.retransmits_up.average();

        if let Some(throughput_up) = throughput_up {
            self.throughput_up_moving_average.add(throughput_up);
        }
        if let Some(retransmits_up) = retransmits_up {
            self.retransmits_up_moving_average.add(retransmits_up);
        }
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

    fn recommendations_download(&mut self, recommendations: &mut Vec<(Recommendation, String)>) {
        let saturation_max = SaturationLevel::from_throughput(
            self.current_throughput.0,
            self.config.max_download_mbps as f64,
        );
        let saturation_current = SaturationLevel::from_throughput(
            self.current_throughput.0,
            self.queue_download_mbps as f64,
        );
        let retransmit_state = RetransmitState::new(
            &self.retransmits_down_moving_average,
            &self.retransmits_down,
        );
        let abs_retransmit = self.retransmits_down_moving_average.average();
        let rtt_state = RttState::new(&self.round_trip_time_moving_average, &self.round_trip_time);

        let params = RecommendationParams {
            direction: RecommendationDirection::Download,
            can_increase: self.queue_download_mbps < self.config.max_download_mbps,
            can_decrease: self.queue_download_mbps > self.config.min_download_mbps,
            saturation_max,
            saturation_current,
            retransmit_state,
            rtt_state,
            abs_retransmit,
        };

        self.recommendation_matrix(recommendations, &params);
    }

    fn recommendations_upload(&mut self, recommendations: &mut Vec<(Recommendation, String)>) {
        let saturation_max = SaturationLevel::from_throughput(
            self.current_throughput.1,
            self.config.max_upload_mbps as f64,
        );
        let saturation_current = SaturationLevel::from_throughput(
            self.current_throughput.1,
            self.queue_upload_mbps as f64,
        );
        let retransmit_state =
            RetransmitState::new(&self.retransmits_up_moving_average, &self.retransmits_up);
        let abs_retransmit = self.retransmits_up_moving_average.average();
        let rtt_state = RttState::new(&self.round_trip_time_moving_average, &self.round_trip_time);

        let params = RecommendationParams {
            direction: RecommendationDirection::Upload,
            can_increase: self.queue_upload_mbps < self.config.max_upload_mbps,
            can_decrease: self.queue_upload_mbps > self.config.min_upload_mbps,
            saturation_max,
            saturation_current,
            retransmit_state,
            rtt_state,
            abs_retransmit,
        };

        self.recommendation_matrix(recommendations, &params);
    }

    pub fn recommendations(&mut self, recommendations: &mut Vec<(Recommendation, String)>) {
        if self.download_state == StormguardState::Running {
            self.recommendations_download(recommendations);
            self.ticks_since_last_probe_download += 1;
            self.ticks_since_last_probe_download =
                u32::min(20, self.ticks_since_last_probe_download);
        }
        if self.upload_state == StormguardState::Running {
            self.recommendations_upload(recommendations);
            self.ticks_since_last_probe_upload += 1;
            self.ticks_since_last_probe_upload = u32::min(20, self.ticks_since_last_probe_upload);
        }
    }
}
