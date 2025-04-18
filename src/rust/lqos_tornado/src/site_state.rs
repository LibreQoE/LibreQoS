mod tornado_state;
mod ring_buffer;
mod recommendation;
mod site;
mod analysis;

use std::collections::HashMap;
use tracing::{debug, info, warn};
use lqos_bus::{BusRequest, BusResponse};
use lqos_queue_tracker::QUEUE_STRUCTURE;
use crate::config::TornadoConfig;
use crate::datalog::LogCommand;
use crate::{MOVING_AVERAGE_BUFFER_SIZE, READING_ACCUMULATOR_SIZE};
use crate::site_state::recommendation::{Recommendation, RecommendationAction, RecommendationDirection};
use crate::site_state::ring_buffer::RingBuffer;
use crate::site_state::site::SiteState;
use crate::site_state::tornado_state::TornadoState;

pub struct SiteStateTracker<'a> {
    sites: HashMap<String, SiteState<'a>>,
}

impl<'a> SiteStateTracker<'a> {
    pub fn from_config(config: &'a TornadoConfig) -> Self {
        let mut sites = HashMap::new();
        for (name, site) in &config.sites {
            sites.insert(
                name.clone(),
                SiteState {
                    config: site,
                    download_state: TornadoState::Warmup,
                    upload_state: TornadoState::Warmup,
                    throughput_down: RingBuffer::new(READING_ACCUMULATOR_SIZE),
                    throughput_up: RingBuffer::new(READING_ACCUMULATOR_SIZE),
                    retransmits_down: RingBuffer::new(READING_ACCUMULATOR_SIZE),
                    retransmits_up: RingBuffer::new(READING_ACCUMULATOR_SIZE),
                    round_trip_time: RingBuffer::new(READING_ACCUMULATOR_SIZE),
                    throughput_down_moving_average: RingBuffer::new(MOVING_AVERAGE_BUFFER_SIZE),
                    throughput_up_moving_average: RingBuffer::new(MOVING_AVERAGE_BUFFER_SIZE),
                    retransmits_down_moving_average: RingBuffer::new(MOVING_AVERAGE_BUFFER_SIZE),
                    retransmits_up_moving_average: RingBuffer::new(MOVING_AVERAGE_BUFFER_SIZE),
                    round_trip_time_moving_average: RingBuffer::new(MOVING_AVERAGE_BUFFER_SIZE),
                    queue_download_mbps: site.max_download_mbps,
                    queue_upload_mbps: site.max_upload_mbps,
                    current_throughput: (0.0, 0.0),
                    ticks_since_last_probe_download: 0,
                    ticks_since_last_probe_upload: 0,
                },
            );
        }
        Self { sites }
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

    pub fn recommendations(&mut self) -> Vec<(Recommendation, String)> {
        let mut recommendations = Vec::new();
        self.sites.iter_mut().for_each(|(_,s)| s.recommendations(&mut recommendations));
        recommendations
    }

    pub fn apply_recommendations(
        &mut self,
        recommendations: Vec<(Recommendation, String)>,
        config: &TornadoConfig,
        log_sender: std::sync::mpsc::Sender<LogCommand>,
    ) {
        // We'll need the queues to apply HTB commands
        let Some(queues) = &QUEUE_STRUCTURE.load().maybe_queues else {
            info!("No queue structure - cannot get stats");
            return;
        };

        for (recommendation, summary) in recommendations {
            // Find the Site Object
            let Some(site) = self.sites.get_mut(&recommendation.site) else {
                continue;
            };
            // Find the Site Config
            let Some(site_config) = config.sites.get(&recommendation.site) else {
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

            let change_rate = match recommendation.direction {
                RecommendationDirection::Download => site_config.step_download_mbps,
                RecommendationDirection::Upload => site_config.step_upload_mbps,
            } as f64;

            let max_rate = match recommendation.direction {
                RecommendationDirection::Download => site_config.max_download_mbps,
                RecommendationDirection::Upload => site_config.max_upload_mbps,
            } as f64;

            let new_rate = match recommendation.action {
                RecommendationAction::IncreaseFast => current_rate + (change_rate * 2.0),
                RecommendationAction::Increase => current_rate + change_rate,
                RecommendationAction::Decrease => current_rate - change_rate,
                RecommendationAction::DecreaseFast => current_rate - (change_rate * 2.0),
            };

            // Are we allowed to do it?
            if new_rate > max_rate {
                continue;
            }

            // Apply the new rate to the QUEUE object
            let new_rate = u64::max(4, new_rate as u64);
            if new_rate == current_rate as u64 {
                // No change
                continue;
            }
            
            let cooldown_secs = match recommendation.action {
                RecommendationAction::IncreaseFast => READING_ACCUMULATOR_SIZE as f32,
                RecommendationAction::Increase => READING_ACCUMULATOR_SIZE as f32 * 0.5,
                RecommendationAction::Decrease => READING_ACCUMULATOR_SIZE as f32 * 0.5,
                RecommendationAction::DecreaseFast => READING_ACCUMULATOR_SIZE as f32,
            };

            // Apply to the site
            match recommendation.direction {
                RecommendationDirection::Download => {
                    site.queue_download_mbps = new_rate;
                    site.ticks_since_last_probe_download = 0;
                }
                RecommendationDirection::Upload => {
                    site.queue_upload_mbps = new_rate;
                    site.ticks_since_last_probe_upload = 0;
                }
            }

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
                format!("{}mbit",new_rate-1),
                "ceil".to_string(),
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
            match recommendation.direction {
                RecommendationDirection::Download => {
                    site.download_state = TornadoState::Cooldown {
                        start: std::time::Instant::now(),
                        duration_secs: cooldown_secs,
                    };
                }
                RecommendationDirection::Upload => {
                    site.upload_state = TornadoState::Cooldown {
                        start: std::time::Instant::now(),
                        duration_secs: cooldown_secs,
                    };
                }
            }

            // Report
            let _ = log_sender.send(LogCommand::SpeedChange {
                site: recommendation.site.clone(),
                download: site.queue_download_mbps,
                upload: site.queue_upload_mbps,
                state: summary,
            });
        }
    }
}