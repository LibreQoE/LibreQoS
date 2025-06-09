mod stormguard_state;
mod ring_buffer;
mod recommendation;
mod site;
mod analysis;

use std::collections::HashMap;
use tracing::{debug, info, warn};
use lqos_bus::{BusRequest, BusResponse};
use lqos_queue_tracker::QUEUE_STRUCTURE;
use lqos_bakery::BakeryCommands;
use lqos_utils::hash_to_i64;
use crate::config::StormguardConfig;
use crate::datalog::LogCommand;
use crate::{MOVING_AVERAGE_BUFFER_SIZE, READING_ACCUMULATOR_SIZE};
use crate::site_state::recommendation::{Recommendation, RecommendationAction, RecommendationDirection};
use crate::site_state::ring_buffer::RingBuffer;
use crate::site_state::site::SiteState;
use crate::site_state::stormguard_state::StormguardState;

pub struct SiteStateTracker<'a> {
    sites: HashMap<String, SiteState<'a>>,
}

impl<'a> SiteStateTracker<'a> {
    pub fn from_config(config: &'a StormguardConfig) -> Self {
        let mut sites = HashMap::new();
        for (name, site) in &config.sites {
            sites.insert(
                name.clone(),
                SiteState {
                    config: site,
                    download_state: StormguardState::Warmup,
                    upload_state: StormguardState::Warmup,
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

    pub fn check_state(&mut self, stats: &crate::StormguardStats) {
        // Update total sites managed
        stats.total_sites_managed.store(self.sites.len() as u64, std::sync::atomic::Ordering::Relaxed);
        
        // Reset state counters
        stats.sites_in_warmup.store(0, std::sync::atomic::Ordering::Relaxed);
        stats.sites_in_cooldown.store(0, std::sync::atomic::Ordering::Relaxed);
        stats.sites_active.store(0, std::sync::atomic::Ordering::Relaxed);
        
        self.sites.iter_mut().for_each(|(_, s)| {
            s.check_state();
            stats.sites_evaluated.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            
            // Count states
            use crate::site_state::stormguard_state::StormguardState;
            match &s.download_state {
                StormguardState::Warmup => stats.sites_in_warmup.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                StormguardState::Cooldown { .. } => stats.sites_in_cooldown.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                StormguardState::Running => stats.sites_active.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            };
        });
    }

    pub fn recommendations(&mut self) -> Vec<(Recommendation, String)> {
        let mut recommendations = Vec::new();
        self.sites.iter_mut().for_each(|(_,s)| s.recommendations(&mut recommendations));
        recommendations
    }

    pub fn apply_recommendations(
        &mut self,
        recommendations: Vec<(Recommendation, String)>,
        config: &StormguardConfig,
        log_sender: std::sync::mpsc::Sender<LogCommand>,
        bakery_sender: crossbeam_channel::Sender<lqos_bakery::BakeryCommands>,
        stats: &crate::StormguardStats,
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

            // Find the Queue Object to verify it exists
            let Some(_queue) = queues.iter().find(|n| {
                if let Some(q) = &n.name {
                    *q == recommendation.site
                } else {
                    false
                }
            }) else {
                info!("Queue {} not found in queue structure", recommendation.site);
                continue;
            };

            // We no longer need interface or class_id since the bakery handles TC commands

            // Find the new bandwidth
            let current_rate = match recommendation.direction {
                RecommendationDirection::Download => site.queue_download_mbps,
                RecommendationDirection::Upload => site.queue_upload_mbps,
            } as f64;

            let max_rate = match recommendation.direction {
                RecommendationDirection::Download => site_config.max_download_mbps,
                RecommendationDirection::Upload => site_config.max_upload_mbps,
            } as f64;
            let min_rate = match recommendation.direction {
                RecommendationDirection::Download => site_config.min_download_mbps,
                RecommendationDirection::Upload => site_config.min_upload_mbps,
            } as f64;

            let new_rate_multiplier = match recommendation.action {
                RecommendationAction::Increase => 1.05,
                RecommendationAction::IncreaseFast => 1.12,
                RecommendationAction::Decrease => 0.95,
                RecommendationAction::DecreaseFast => 0.88,
            };
            let new_rate = match recommendation.direction {
                RecommendationDirection::Download => site.queue_download_mbps,
                RecommendationDirection::Upload => site.queue_upload_mbps,
            } as f64 * new_rate_multiplier;
            let new_rate = new_rate.round();

            // Are we allowed to do it?
            if new_rate > max_rate {
                continue;
            }
            if new_rate < min_rate {
                continue;
            }

            // Apply the new rate to the QUEUE object
            let new_rate = u64::max(4, new_rate as u64);
            if new_rate == current_rate as u64 {
                // No change
                continue;
            }
            
            let cooldown_secs = match recommendation.action {
                RecommendationAction::IncreaseFast => (READING_ACCUMULATOR_SIZE as f32 * 0.1).max(2.0),
                RecommendationAction::Increase => (READING_ACCUMULATOR_SIZE as f32 * 0.05).max(1.0),
                RecommendationAction::Decrease => READING_ACCUMULATOR_SIZE as f32 * 0.5,
                RecommendationAction::DecreaseFast => READING_ACCUMULATOR_SIZE as f32,
            };
            debug!("Cooldown for {:?} set to {:.1}s", recommendation.action, cooldown_secs);

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

            // Apply for dependents
            for dependent in &site.config.dependent_nodes {
                let max_rate = match recommendation.direction {
                    RecommendationDirection::Download => dependent.original_max_download_mbps,
                    RecommendationDirection::Upload => dependent.original_max_upload_mbps,
                };
                if max_rate < new_rate {
                    continue;
                }
                info!("Applying rate change to dependent {}: {} -> {}", dependent.name, dependent.original_max_download_mbps, new_rate);
                // Send bakery command for dependent site
                let dependent_site_hash = hash_to_i64(&dependent.name);
                let (download_min, upload_min, download_max, upload_max) = match recommendation.direction {
                    RecommendationDirection::Download => (new_rate as f32 - 1.0, dependent.original_max_upload_mbps as f32 - 1.0, new_rate as f32, dependent.original_max_upload_mbps as f32),
                    RecommendationDirection::Upload => (dependent.original_max_download_mbps as f32 - 1.0, new_rate as f32 - 1.0, dependent.original_max_download_mbps as f32, new_rate as f32),
                };
                if let Err(e) = bakery_sender.try_send(BakeryCommands::ChangeSiteSpeedLive {
                    site_hash: dependent_site_hash,
                    download_bandwidth_min: download_min,
                    upload_bandwidth_min: upload_min,
                    download_bandwidth_max: download_max,
                    upload_bandwidth_max: upload_max,
                }) {
                    warn!("Failed to send bakery command for dependent {}: {}", dependent.name, e);
                }
            }

            // Track adjustment direction
            match recommendation.action {
                RecommendationAction::Increase | RecommendationAction::IncreaseFast => {
                    stats.adjustments_up.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                RecommendationAction::Decrease | RecommendationAction::DecreaseFast => {
                    stats.adjustments_down.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }

            // Actually make the change via bakery
            let site_hash = hash_to_i64(&recommendation.site);
            let (download_min, upload_min, download_max, upload_max) = match recommendation.direction {
                RecommendationDirection::Download => {
                    (new_rate as f32 - 1.0, site.queue_upload_mbps as f32 - 1.0, new_rate as f32, site.queue_upload_mbps as f32)
                },
                RecommendationDirection::Upload => {
                    (site.queue_download_mbps as f32 - 1.0, new_rate as f32 - 1.0, site.queue_download_mbps as f32, new_rate as f32)
                },
            };
            if let Err(e) = bakery_sender.try_send(BakeryCommands::ChangeSiteSpeedLive {
                site_hash,
                download_bandwidth_min: download_min,
                upload_bandwidth_min: upload_min,
                download_bandwidth_max: download_max,
                upload_bandwidth_max: upload_max,
            }) {
                warn!("Failed to send bakery command for site {}: {}", recommendation.site, e);
            }

            // Finish Up by entering cooldown
            debug!("Recommendation applied: entering cooldown");
            match recommendation.direction {
                RecommendationDirection::Download => {
                    site.download_state = StormguardState::Cooldown {
                        start: std::time::Instant::now(),
                        duration_secs: cooldown_secs,
                    };
                }
                RecommendationDirection::Upload => {
                    site.upload_state = StormguardState::Cooldown {
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
