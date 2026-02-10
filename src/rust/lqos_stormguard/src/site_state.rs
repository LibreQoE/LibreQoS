mod analysis;
mod recommendation;
mod ring_buffer;
mod site;
mod stormguard_state;

use crate::config::StormguardConfig;
use crate::site_state::analysis::SaturationLevel;
use crate::datalog::LogCommand;
use crate::site_state::recommendation::{
    Recommendation, RecommendationAction, RecommendationDirection,
};
use crate::site_state::ring_buffer::RingBuffer;
use crate::site_state::site::SiteState;
use crate::site_state::stormguard_state::StormguardState;
use crate::{MOVING_AVERAGE_BUFFER_SIZE, READING_ACCUMULATOR_SIZE};
use crossbeam_channel::Sender;
use lqos_bakery::BakeryCommands;
use lqos_bus::{
    BusRequest, BusResponse, StormguardDebugDirection, StormguardDebugEntry, TcHandle,
};
use lqos_queue_tracker::QUEUE_STRUCTURE;
use std::collections::HashMap;
use tracing::{debug, info, warn};

pub struct SiteStateTracker {
    sites: HashMap<String, SiteState>,
}

impl SiteStateTracker {
    pub fn from_config(config: &StormguardConfig) -> Self {
        let mut sites = HashMap::new();
        for (name, site) in &config.sites {
            sites.insert(
                name.clone(),
                SiteState {
                    config: site.clone(),
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
        let requests = vec![BusRequest::GetFullNetworkMap];
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
                    let retransmits_down = node_info.current_retransmits.0 as f64
                        / node_info.current_tcp_packets.0 as f64;
                    target.retransmits_down.add(retransmits_down);
                }
                if node_info.current_tcp_packets.1 > 0 {
                    let retransmits_up = node_info.current_retransmits.1 as f64
                        / node_info.current_tcp_packets.1 as f64;
                    target.retransmits_up.add(retransmits_up);
                }

                // Round-Trip Time
                if !node_info.rtts.is_empty() {
                    let mut my_round_trip_times = node_info.rtts.clone();
                    my_round_trip_times.sort_by(|a, b| a.total_cmp(b));
                    let samples = my_round_trip_times.len();
                    let p90 = my_round_trip_times[(samples as f32 * 0.9) as usize];
                    target.round_trip_time.add(p90 as f64);
                }
            }
        }
    }

    pub fn check_state(&mut self) {
        self.sites.iter_mut().for_each(|(_, s)| s.check_state());
    }

    pub fn recommendations(&mut self) -> Vec<(Recommendation, String)> {
        let mut recommendations = Vec::new();
        self.sites
            .iter_mut()
            .for_each(|(_, s)| s.recommendations(&mut recommendations));
        recommendations
    }

    pub fn debug_snapshot(&self, config: &StormguardConfig) -> Vec<StormguardDebugEntry> {
        self.sites
            .iter()
            .filter_map(|(name, site)| {
                let Some(site_config) = config.sites.get(name) else {
                    return None;
                };

                let make_direction =
                    |direction: RecommendationDirection| -> StormguardDebugDirection {
                        let (queue_mbps, min_mbps, max_mbps, throughput_mbps, throughput_ma_mbps, retrans, retrans_ma) = match direction {
                            RecommendationDirection::Download => (
                                site.queue_download_mbps,
                                site_config.min_download_mbps,
                                site_config.max_download_mbps,
                                site.current_throughput.0,
                                site.throughput_down_moving_average.average(),
                                site.retransmits_down.average(),
                                site.retransmits_down_moving_average.average(),
                            ),
                            RecommendationDirection::Upload => (
                                site.queue_upload_mbps,
                                site_config.min_upload_mbps,
                                site_config.max_upload_mbps,
                                site.current_throughput.1,
                                site.throughput_up_moving_average.average(),
                                site.retransmits_up.average(),
                                site.retransmits_up_moving_average.average(),
                            ),
                        };

                        let (state, cooldown_remaining_secs) = match direction {
                            RecommendationDirection::Download => match &site.download_state {
                                StormguardState::Warmup => ("Warmup".to_string(), None),
                                StormguardState::Running => ("Running".to_string(), None),
                                StormguardState::Cooldown {
                                    start,
                                    duration_secs,
                                } => {
                                    let elapsed = start.elapsed().as_secs_f32();
                                    let remaining = (duration_secs - elapsed).max(0.0);
                                    ("Cooldown".to_string(), Some(remaining))
                                }
                            },
                            RecommendationDirection::Upload => match &site.upload_state {
                                StormguardState::Warmup => ("Warmup".to_string(), None),
                                StormguardState::Running => ("Running".to_string(), None),
                                StormguardState::Cooldown {
                                    start,
                                    duration_secs,
                                } => {
                                    let elapsed = start.elapsed().as_secs_f32();
                                    let remaining = (duration_secs - elapsed).max(0.0);
                                    ("Cooldown".to_string(), Some(remaining))
                                }
                            },
                        };

                        let saturation_max = SaturationLevel::from_throughput(
                            throughput_mbps,
                            match direction {
                                RecommendationDirection::Download => {
                                    site_config.max_download_mbps as f64
                                }
                                RecommendationDirection::Upload => site_config.max_upload_mbps as f64,
                            },
                        );
                        let saturation_current = SaturationLevel::from_throughput(
                            throughput_mbps,
                            queue_mbps as f64,
                        );

                        let can_increase = queue_mbps < max_mbps;
                        let can_decrease = queue_mbps > min_mbps;

                        StormguardDebugDirection {
                            queue_mbps,
                            min_mbps,
                            max_mbps,
                            throughput_mbps,
                            throughput_ma_mbps,
                            retrans,
                            retrans_ma,
                            rtt: site.round_trip_time.average(),
                            rtt_ma: site.round_trip_time_moving_average.average(),
                            state,
                            cooldown_remaining_secs,
                            saturation_current: saturation_current.to_string(),
                            saturation_max: saturation_max.to_string(),
                            can_increase,
                            can_decrease,
                        }
                    };

                Some(StormguardDebugEntry {
                    site: name.clone(),
                    download: make_direction(RecommendationDirection::Download),
                    upload: make_direction(RecommendationDirection::Upload),
                })
            })
            .collect()
    }

    pub fn apply_recommendations(
        &mut self,
        recommendations: Vec<(Recommendation, String)>,
        config: &StormguardConfig,
        log_sender: std::sync::mpsc::Sender<LogCommand>,
        bakery_sender: Sender<BakeryCommands>,
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
            // Skip circuit-level queues that host a qdisc (CAKE/fq_codel). Changing HTB at that
            // level can deadlock the kernel; only adjust parent branch nodes.
            if queue.circuit_id.is_some() {
                warn!(
                    "StormGuard skipped {} because it resolves to a circuit queue (qdisc host).",
                    recommendation.site
                );
                continue;
            }

            // Find the interface
            let interface_name = match recommendation.direction {
                RecommendationDirection::Download => config.download_interface.clone(),
                RecommendationDirection::Upload => config.upload_interface.clone(),
            };

            // Find the TC class
            let class_handle = queue.class_id;

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
                RecommendationAction::Increase => 1.15,
                RecommendationAction::IncreaseFast => 1.30,
                RecommendationAction::Decrease => 0.95,
                RecommendationAction::DecreaseFast => 0.88,
            };
            let new_rate = match recommendation.direction {
                RecommendationDirection::Download => site.queue_download_mbps,
                RecommendationDirection::Upload => site.queue_upload_mbps,
            } as f64
                * new_rate_multiplier;
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
                RecommendationAction::IncreaseFast => {
                    // Halved to allow quicker follow-up increases.
                    (READING_ACCUMULATOR_SIZE as f32 * 0.05).max(2.0)
                }
                RecommendationAction::Increase => {
                    (READING_ACCUMULATOR_SIZE as f32 * 0.025).max(1.0)
                }
                RecommendationAction::Decrease => READING_ACCUMULATOR_SIZE as f32 * 0.25,
                RecommendationAction::DecreaseFast => READING_ACCUMULATOR_SIZE as f32 * 0.5,
            };
            debug!(
                "Cooldown for {:?} set to {:.1}s",
                recommendation.action, cooldown_secs
            );

            // Apply to the site
            match recommendation.direction {
                RecommendationDirection::Download => {
                    site.queue_download_mbps = new_rate;
                    site.ticks_since_last_probe_download = 0;
                    let mut lock = crate::STORMGUARD_STATS.lock();
                    if let Some(site) = lock.iter_mut().find(|(n, _, _)| n == &recommendation.site)
                    {
                        site.1 = new_rate;
                    }
                }
                RecommendationDirection::Upload => {
                    site.queue_upload_mbps = new_rate;
                    site.ticks_since_last_probe_upload = 0;
                    let mut lock = crate::STORMGUARD_STATS.lock();
                    if let Some(site) = lock.iter_mut().find(|(n, _, _)| n == &recommendation.site)
                    {
                        site.2 = new_rate;
                    }
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
                info!(
                    "Applying rate change to dependent {}: {} -> {}",
                    dependent.name, dependent.original_max_download_mbps, new_rate
                );
                Self::apply_htb_change(
                    config,
                    &interface_name,
                    dependent.class_id,
                    new_rate,
                    bakery_sender.clone(),
                );
            }

            // Actually make the change
            Self::apply_htb_change(
                config,
                &interface_name,
                class_handle,
                new_rate,
                bakery_sender.clone(),
            );

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

    fn apply_htb_change(
        config: &StormguardConfig,
        interface_name: &str,
        class_handle: TcHandle,
        new_rate: u64,
        bakery_sender: Sender<BakeryCommands>,
    ) {
        if let Err(e) = bakery_sender.send(BakeryCommands::StormGuardAdjustment {
            dry_run: config.dry_run,
            interface_name: interface_name.to_owned(),
            class_id: class_handle.as_tc_string(),
            new_rate,
        }) {
            warn!("Failed to send StormGuard adjustment command: {}", e);
            return;
        }

        // // Build the HTB command
        // let args = vec![
        //     "class".to_string(),
        //     "change".to_string(),
        //     "dev".to_string(),
        //     interface_name.to_string(),
        //     "classid".to_string(),
        //     class_id.to_string(),
        //     "htb".to_string(),
        //     "rate".to_string(),
        //     format!("{}mbit", new_rate - 1),
        //     "ceil".to_string(),
        //     format!("{}mbit", new_rate),
        // ];
        // if config.dry_run {
        //     warn!("DRY RUN: /sbin/tc {}", args.join(" "));
        // } else {
        //     let output = std::process::Command::new("/sbin/tc")
        //         .args(&args)
        //         .output();
        //     match output {
        //         Err(e) => {
        //             warn!("Failed to run tc command: {}", e);
        //         }
        //         Ok(out) => {
        //             if !out.status.success() {
        //                 warn!("tc command failed: {}", String::from_utf8_lossy(&out.stderr));
        //             } else {
        //                 info!("tc command succeeded: {}", String::from_utf8_lossy(&out.stdout));
        //             }
        //         }
        //     }
        // }
    }
}
