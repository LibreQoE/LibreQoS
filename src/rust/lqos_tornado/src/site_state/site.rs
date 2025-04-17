use tracing::{debug, info};
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

    fn recommend(
        &self,
        below_max: bool,
        current_throughput: f64,
        max_mbps: f64,
        recommendations: &mut Vec<Recommendation>,
        direction: RecommendationDirection,
    ) {
        if current_throughput < f64::min(max_mbps / 10.0, 10.0) {
            if below_max {
                info!("{} is below 10% of max bandwidth ({}), try increase", direction, current_throughput);
                recommendations.push(
                    Recommendation {
                        site: self.name.clone(),
                        direction,
                        action: RecommendationAction::Increase,
                    }
                );
            }
            return;
        }
        let tcp_retransmits_ma = self.retransmits_up_moving_average.average().unwrap_or(1.0);
        let tcp_retransmits_avg = self.retransmits_up.average().unwrap_or(1.0);
        let tcp_retransmits_relative = tcp_retransmits_avg / tcp_retransmits_ma;
        if tcp_retransmits_relative < 0.8 {
            //info!("TCP Retransmits ({}) are Improving! Magnitude {:.2}", direction, tcp_retransmits_relative);
            if below_max {
                recommendations.push(
                    Recommendation {
                        site: self.name.clone(),
                        direction,
                        action: RecommendationAction::Increase,
                    }
                );
            }
        } else if tcp_retransmits_relative > 1.2 {
            //info!("TCP Retransmits ({}) are Deteriorating! Magnitude {:.2}", direction, tcp_retransmits_relative);
            recommendations.push(
                Recommendation {
                    site: self.name.clone(),
                    direction,
                    action: RecommendationAction::Decrease,
                }
            );
        }
    }

    fn recommendations_download(&self, recommendations: &mut Vec<Recommendation>) {
        self.recommend(
            self.queue_download_mbps < self.max_download_mbps,
            self.throughput_down_moving_average.average().unwrap_or(0.0),
            self.max_download_mbps as f64,
            recommendations,
            RecommendationDirection::Download,
        );
    }

    fn recommendations_upload(&self, recommendations: &mut Vec<Recommendation>) {
        self.recommend(
            self.queue_upload_mbps < self.max_upload_mbps,
            self.throughput_up_moving_average.average().unwrap_or(0.0),
            self.max_upload_mbps as f64,
            recommendations,
            RecommendationDirection::Upload,
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
