use tracing::{debug, info};
use crate::site_state::analysis::{RetransmitState, RttState, SaturationLevel};
use crate::site_state::recommendation::{Recommendation, RecommendationAction, RecommendationDirection};
use crate::site_state::ring_buffer::RingBuffer;
use crate::site_state::tornado_state::TornadoState;

pub struct SiteState {
    pub name: String,
    pub max_download_mbps: u64,
    pub max_upload_mbps: u64,
    pub state: TornadoState,

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
}

impl SiteState {
    pub fn check_state(&mut self) {
        match self.state {
            TornadoState::Warmup => {
                // Do we have enough data to consider ourselves functional?
                let throughput_down_count = self.throughput_down.count();
                let throughput_up_count = self.throughput_up.count();
                let retransmits_down_count = self.retransmits_down.count();
                let retransmits_up_count = self.retransmits_up.count();
                if throughput_down_count > 10 && throughput_up_count > 10 && retransmits_down_count > 10 && retransmits_up_count > 10 {
                    info!("Site {} has completed warm-up.", self.name);
                    self.state = TornadoState::Running;
                }
                return;
            }
            TornadoState::Running => {
                self.moving_averages();
                return;
            }
            TornadoState::Cooldown(start) => {
                self.moving_averages();

                // Check if cooldown period is over
                let now = std::time::Instant::now();
                if now.duration_since(start).as_secs() > 5 {
                    debug!("Site {} has completed cooldown.", self.name);
                    self.state = TornadoState::Running;
                    return;
                }
            }
        }
    }

    pub fn moving_averages(&mut self) {
        let throughput_down = self.throughput_down.average();
        let throughput_up = self.throughput_up.average();
        let retransmits_down = self.retransmits_down.average();
        let retransmits_up = self.retransmits_up.average();
        let round_trip_time = self.round_trip_time.average();

        if let Some(throughput_down) = throughput_down {
            self.throughput_down_moving_average.add(throughput_down);
        }
        if let Some(throughput_up) = throughput_up {
            self.throughput_up_moving_average.add(throughput_up);
        }
        if let Some(retransmits_down) = retransmits_down {
            self.retransmits_down_moving_average.add(retransmits_down);
        }
        if let Some(retransmits_up) = retransmits_up {
            self.retransmits_up_moving_average.add(retransmits_up);
        }
        if let Some(round_trip_time) = round_trip_time {
            self.round_trip_time_moving_average.add(round_trip_time);
        }
    }

    fn recommendation_matrix(
        &self,
        recommendations: &mut Vec<Recommendation>,
        direction: RecommendationDirection,
        saturation_max: SaturationLevel, // Saturation relative to the max bandwidth
        saturation_current: SaturationLevel, // Saturation relative to the current bandwidth
        retransmit_state: RetransmitState,
        rtt_state: RttState,
    ) {
        if saturation_current == SaturationLevel::High || saturation_max == SaturationLevel::High {
            if retransmit_state == RetransmitState::High || retransmit_state == RetransmitState::RisingFast {
                recommendations.push(Recommendation::new(&self.name, direction, RecommendationAction::DecreaseFast));
                return; // Only 1 recommendation!
            }
            if retransmit_state == RetransmitState::Rising {
                recommendations.push(Recommendation::new(&self.name, direction, RecommendationAction::Decrease));
                return; // Only 1 recommendation!
            }
            if retransmit_state == RetransmitState::FallingFast || retransmit_state == RetransmitState::Falling {
                recommendations.push(Recommendation::new(&self.name, direction, RecommendationAction::Increase));
                return; // Only 1 recommendation!
            }
            if rtt_state == RttState::Rising {
                recommendations.push(Recommendation::new(&self.name, direction, RecommendationAction::Decrease));
                return; // Only 1 recommendation!
            }
            if rtt_state == RttState::Falling {
                recommendations.push(Recommendation::new(&self.name, direction, RecommendationAction::Increase));
                return; // Only 1 recommendation!
            }
        } else if saturation_current == SaturationLevel::Medium || saturation_max == SaturationLevel::Medium {
            if retransmit_state == RetransmitState::High || retransmit_state == RetransmitState::RisingFast {
                recommendations.push(Recommendation::new(&self.name, direction, RecommendationAction::Decrease));
                return; // Only 1 recommendation!
            }
            if retransmit_state == RetransmitState::FallingFast || retransmit_state == RetransmitState::Falling {
                recommendations.push(Recommendation::new(&self.name, direction, RecommendationAction::Increase));
                return; // Only 1 recommendation!
            }
        } else {
            // We're in Low saturation
            if retransmit_state == RetransmitState::Low || retransmit_state == RetransmitState::Falling || retransmit_state == RetransmitState::FallingFast {
                recommendations.push(Recommendation::new(&self.name, direction, RecommendationAction::IncreaseFast));
                return; // Only 1 recommendation!
            }
            if retransmit_state == RetransmitState::High || retransmit_state == RetransmitState::Rising || retransmit_state == RetransmitState::RisingFast {
                recommendations.push(Recommendation::new(&self.name, direction, RecommendationAction::Decrease));
                return; // Only 1 recommendation!
            }
        }
    }

    fn recommendations_download(&self, recommendations: &mut Vec<Recommendation>) {
        let saturation_max = SaturationLevel::from_throughput(
            self.current_throughput.0,
            self.max_download_mbps as f64,
        );
        let saturation_current = SaturationLevel::from_throughput(
            self.current_throughput.0,
            self.queue_download_mbps as f64,
        );
        let retransmit_state = RetransmitState::new(
            &self.retransmits_down_moving_average,
            &self.retransmits_down,
        );
        let rtt_state = RttState::new(
            &self.round_trip_time_moving_average,
            &self.round_trip_time,
        );

        self.recommendation_matrix(
            recommendations,
            RecommendationDirection::Download,
            saturation_max,
            saturation_current,
            retransmit_state,
            rtt_state,
        );
    }

    fn recommendations_upload(&self, recommendations: &mut Vec<Recommendation>) {
        let saturation_max = SaturationLevel::from_throughput(
            self.current_throughput.1,
            self.max_upload_mbps as f64,
        );
        let saturation_current = SaturationLevel::from_throughput(
            self.current_throughput.1,
            self.queue_upload_mbps as f64,
        );
        let retransmit_state = RetransmitState::new(
            &self.retransmits_up_moving_average,
            &self.retransmits_up,
        );
        let rtt_state = RttState::new(
            &self.round_trip_time_moving_average,
            &self.round_trip_time,
        );

        self.recommendation_matrix(
            recommendations,
            RecommendationDirection::Upload,
            saturation_max,
            saturation_current,
            retransmit_state,
            rtt_state,
        );
    }

    pub fn recommendations(&self, recommendations: &mut Vec<Recommendation>) {
        if self.state != TornadoState::Running {
            return;
        }
        self.recommendations_download(recommendations);
        self.recommendations_upload(recommendations);
    }
}
