mod tornado_state;
mod ring_buffer;

use std::collections::HashMap;
use std::fmt::Display;
use tracing::{debug, info, warn};
use lqos_bus::{BusRequest, BusResponse};
use lqos_queue_tracker::QUEUE_STRUCTURE;
use crate::config::TornadoConfig;
use crate::site_state::ring_buffer::RingBuffer;
use crate::site_state::tornado_state::TornadoState;

#[derive(Debug)]
pub struct Recommendation {
    site: String,
    direction: RecommendationDirection,
    action: RecommendationAction,
}

#[derive(Debug)]
pub enum RecommendationDirection {
    Download,
    Upload,
}

impl Display for RecommendationDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecommendationDirection::Download => write!(f, "Download"),
            RecommendationDirection::Upload => write!(f, "Upload"),
        }
    }
}

#[derive(Debug)]
pub enum RecommendationAction {
    Increase,
    Decrease,
}

pub struct SiteState {
    name: String,
    max_download_mbps: u64,
    max_upload_mbps: u64,
    state: TornadoState,

    // Queue Bandwidth
    queue_download_mbps: u64,
    queue_upload_mbps: u64,
    current_throughput: (f64, f64),

    // Current Data Buffers
    throughput_down: RingBuffer,
    throughput_up: RingBuffer,
    retransmits_down: RingBuffer,
    retransmits_up: RingBuffer,
    round_trip_time: RingBuffer,

    // Moving Average Buffers
    throughput_down_moving_average: RingBuffer,
    throughput_up_moving_average: RingBuffer,
    retransmits_down_moving_average: RingBuffer,
    retransmits_up_moving_average: RingBuffer,
    round_trip_time_moving_average: RingBuffer,
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

pub struct SiteStateTracker {
    sites: HashMap<String, SiteState>,
}

impl SiteStateTracker {
    pub fn from_config(config: &TornadoConfig) -> Self {
        const SHORT_BUFFER_SIZE: usize = 30;
        const LONG_BUFFER_SIZE: usize = 120;
        let mut sites = HashMap::new();
        for (name, site) in &config.sites {
            sites.insert(
                name.clone(),
                SiteState {
                    name: name.clone(),
                    max_download_mbps: site.max_download_mbps,
                    max_upload_mbps: site.max_upload_mbps,
                    state: TornadoState::Warmup,
                    throughput_down: RingBuffer::new(SHORT_BUFFER_SIZE),
                    throughput_up: RingBuffer::new(SHORT_BUFFER_SIZE),
                    retransmits_down: RingBuffer::new(SHORT_BUFFER_SIZE),
                    retransmits_up: RingBuffer::new(SHORT_BUFFER_SIZE),
                    round_trip_time: RingBuffer::new(SHORT_BUFFER_SIZE),
                    throughput_down_moving_average: RingBuffer::new(LONG_BUFFER_SIZE),
                    throughput_up_moving_average: RingBuffer::new(LONG_BUFFER_SIZE),
                    retransmits_down_moving_average: RingBuffer::new(LONG_BUFFER_SIZE),
                    retransmits_up_moving_average: RingBuffer::new(LONG_BUFFER_SIZE),
                    round_trip_time_moving_average: RingBuffer::new(LONG_BUFFER_SIZE),
                    queue_download_mbps: site.max_download_mbps,
                    queue_upload_mbps: site.max_upload_mbps,
                    current_throughput: (0.0, 0.0),
                },
            );
        }
        SiteStateTracker { sites }
    }


    pub async fn read_new_tick_data(&mut self) {
        let requests = vec![
            BusRequest::GetFullNetworkMap,
        ];
        let Ok(responses) = lqos_bus::bus_request(requests).await else {
            info!("Failed to get lqosd stats");
            return;
        };

        for response in responses {
            let BusResponse::NetworkMap(all_nodes) = response else {
                continue;
            };
            for (_, node_info) in all_nodes {
                let Some(target) = self.sites.get_mut(&node_info.name) else {
                    continue;
                };

                // Record throughput if there is any
                if node_info.current_throughput.0 > 0 {
                    let n = (node_info.current_throughput.0 as f64 * 8.0) / 1_000_000.0;
                    target.throughput_down.add(n);
                    target.current_throughput.0 = n;
                }
                if node_info.current_throughput.1 > 0 {
                    let n = (node_info.current_throughput.1 as f64 * 8.0) / 1_000_000.0;
                    target.throughput_up.add(n);
                    target.current_throughput.1 = n;
                }

                // Retransmits (as a percentage of TCP packets)
                if node_info.current_tcp_packets.0 > 0 {
                    let retransmits_down = node_info.current_retransmits.0 as f64 / node_info.current_tcp_packets.0 as f64;
                    target.retransmits_down.add(retransmits_down);
                }
                if node_info.current_tcp_packets.1 > 0 {
                    let retransmits_up = node_info.current_retransmits.1 as f64 / node_info.current_tcp_packets.1 as f64;
                    target.retransmits_up.add(retransmits_up);
                }

                // Round-Trip Time
                if node_info.rtts.len() > 1 {
                    let mut my_round_trip_times = node_info.rtts.clone();
                    my_round_trip_times.sort_by(|a, b| a.partial_cmp(b).unwrap());
                    let samples = my_round_trip_times.len();
                    let p90 = my_round_trip_times[(samples as f32 * 0.9) as usize];
                    target.round_trip_time.add(p90 as f64);
                }
            }
        }
    }

    pub fn check_state(&mut self) {
        self.sites.iter_mut().for_each(|(_,s)| s.check_state());
    }

    pub fn moving_averages(&mut self) {
        self.sites.iter_mut().for_each(|(_,s)| s.moving_averages());
    }

    pub fn recommendations(&self) -> Vec<Recommendation> {
        let mut recommendations = Vec::new();
        self.sites.iter().for_each(|(_,s)| s.recommendations(&mut recommendations));
        recommendations
    }

    pub fn apply_recommendations(&mut self, recommendations: Vec<Recommendation>, config: &TornadoConfig) {
        // We'll need the queues to apply HTB commands
        let Some(queues) = &QUEUE_STRUCTURE.load().maybe_queues else {
            info!("No queue structure - cannot get stats");
            return;
        };

        for recommendation in recommendations {
            // Find the Site Object
            let Some(site) = self.sites.get_mut(&recommendation.site) else {
                continue;
            };

            // Find the Queue Object
            let Some(queue) = queues.iter().find(|n| {
                if let Some(q) = &n.name {
                    *q == recommendation.site
                } else {
                    false
                }
            }) else {
                info!("Queue {} not found in queue structure", recommendation.site);
                continue;
            };

            // Find the interface
            let interface_name = match recommendation.direction {
                RecommendationDirection::Download => config.download_interface.clone(),
                RecommendationDirection::Upload => config.upload_interface.clone(),
            };

            // Find the TC class
            let class_id = queue.class_id.to_string();

            // Find the new bandwidth
            let current_rate = match recommendation.direction {
                RecommendationDirection::Download => site.queue_download_mbps,
                RecommendationDirection::Upload => site.queue_upload_mbps,
            } as f64;
            let change_rate = f64::max(1.0, match recommendation.direction {
                RecommendationDirection::Download => site.max_download_mbps,
                RecommendationDirection::Upload => site.max_upload_mbps,
            } as f64 / 100.0);
            let new_rate = match recommendation.action {
                RecommendationAction::Increase => current_rate + change_rate,
                RecommendationAction::Decrease => current_rate - change_rate,
            };

            // Apply the new rate to the QUEUE object
            let new_rate = u64::max(4, new_rate as u64);
            match recommendation.direction {
                RecommendationDirection::Download => {
                    site.queue_download_mbps = new_rate;
                }
                RecommendationDirection::Upload => {
                    site.queue_upload_mbps = new_rate;
                }
            }
            if new_rate == current_rate as u64 {
                // No change
                continue;
            }
            
            // Report
            info!("Changing rate for site {}/{} from {:.2} mbps to {} mbps",
                recommendation.site,
                recommendation.direction,
                current_rate,
                new_rate
            );

            // Build the HTB command
            let args = vec![
                "class".to_string(),
                "change".to_string(),
                "dev".to_string(),
                interface_name,
                "classid".to_string(),
                class_id.to_string(),
                "htb".to_string(),
                "rate".to_string(),
                format!("{}mbit",new_rate),
            ];
            if config.dry_run {
                warn!("DRY RUN: /sbin/tc {}", args.join(" "));
            } else {
                let output = std::process::Command::new("/sbin/tc")
                    .args(&args)
                    .output();
                match output {
                    Err(e) => {
                        warn!("Failed to run tc command: {}", e);
                    }
                    Ok(out) => {
                        if !out.status.success() {
                            warn!("tc command failed: {}", String::from_utf8_lossy(&out.stderr));
                        } else {
                            info!("tc command succeeded: {}", String::from_utf8_lossy(&out.stdout));
                        }
                    }
                }
            }

            // Finish Up by entering cooldown
            debug!("Recommendation applied: entering cooldown");
            site.state = TornadoState::Cooldown(std::time::Instant::now());
        }
    }
}