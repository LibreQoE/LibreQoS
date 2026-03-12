mod analysis;
mod recommendation;
mod ring_buffer;
mod site;
mod stormguard_state;

use crate::active_ping::TimedRtt;
use crate::adaptive_actions::{
    CircuitFallbackOutcome, SiteOverrideUpdate, apply_circuit_fallback,
    apply_site_override_updates, clear_circuit_fallback, load_persisted_circuit_fallbacks,
};
use crate::config::StormguardConfig;
use crate::datalog::LogCommand;
use crate::site_state::analysis::SaturationLevel;
use crate::site_state::recommendation::{
    Recommendation, RecommendationAction, RecommendationDirection,
};
use crate::site_state::ring_buffer::RingBuffer;
use crate::site_state::site::SiteState;
use crate::site_state::stormguard_state::StormguardState;
use crate::{MOVING_AVERAGE_BUFFER_SIZE, READING_ACCUMULATOR_SIZE};
use crossbeam_channel::Sender;
use lqos_bakery::BakeryCommands;
use lqos_bus::{BusRequest, BusResponse, StormguardDebugDirection, StormguardDebugEntry, TcHandle};
use lqos_queue_tracker::QUEUE_STRUCTURE;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

pub struct SiteStateTracker {
    sites: HashMap<String, SiteState>,
    active_circuit_fallbacks: HashSet<String>,
}

struct CircuitQueueRecommendationContext<'a> {
    active_circuit_fallbacks: &'a mut HashSet<String>,
    site: &'a mut SiteState,
    config: &'a StormguardConfig,
    recommendation: &'a Recommendation,
    summary: &'a str,
    circuit_id: &'a str,
    cooldown_secs: f32,
    log_sender: &'a std::sync::mpsc::Sender<LogCommand>,
    bakery_sender: Sender<BakeryCommands>,
}

impl SiteStateTracker {
    pub fn from_config(config: &StormguardConfig) -> Self {
        let mut sites = HashMap::new();
        for (name, site) in &config.sites {
            let delay_probe = matches!(
                config.strategy,
                lqos_config::StormguardStrategy::DelayProbe
                    | lqos_config::StormguardStrategy::DelayProbeActive
            );
            sites.insert(
                name.clone(),
                SiteState {
                    config: site.clone(),
                    download_state: if delay_probe {
                        StormguardState::Running
                    } else {
                        StormguardState::Warmup
                    },
                    upload_state: if delay_probe {
                        StormguardState::Running
                    } else {
                        StormguardState::Warmup
                    },
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
                    queue_download_mbps: site.current_download_mbps,
                    queue_upload_mbps: site.current_upload_mbps,
                    current_throughput: (0.0, 0.0),
                    current_rtt_ms: None,
                    passive_rtt_ms: None,
                    active_ping_rtt_ms: None,
                    rtt_sample_for_baseline_ms: None,
                    rtt_baseline_ms: None,
                    last_passive_rtt_ms: None,
                    last_passive_rtt_at: None,
                    passive_rtt_updated_this_tick: false,
                    last_action_download: None,
                    last_action_upload: None,
                    ticks_since_last_probe_download: 0,
                    ticks_since_last_probe_upload: 0,
                },
            );
        }
        Self {
            sites,
            active_circuit_fallbacks: HashSet::new(),
        }
    }

    pub fn replay_persisted_adjustments(
        &mut self,
        config: &StormguardConfig,
        bakery_sender: Sender<BakeryCommands>,
    ) {
        if config.dry_run {
            return;
        }

        let Some(queues) = &QUEUE_STRUCTURE.load().maybe_queues else {
            info!("No queue structure - cannot replay StormGuard adjustments");
            return;
        };

        match load_persisted_circuit_fallbacks() {
            Ok(fallbacks) => {
                for (circuit_id, fallback) in fallbacks {
                    self.active_circuit_fallbacks.insert(circuit_id.clone());
                    if let Err(e) = apply_circuit_fallback(
                        &circuit_id,
                        &fallback.sqm_override,
                        false,
                        false,
                        bakery_sender.clone(),
                    ) {
                        warn!(
                            "Failed to replay persisted StormGuard circuit fallback for {}: {}",
                            circuit_id, e
                        );
                    }
                }
            }
            Err(e) => warn!(
                "Failed to load persisted StormGuard circuit fallbacks: {}",
                e
            ),
        }

        for (name, site) in &self.sites {
            let Some(queue) = queues
                .iter()
                .find(|n| n.name.as_deref() == Some(name.as_str()))
            else {
                continue;
            };

            if queue.circuit_id.is_some() {
                continue;
            }

            for direction in [
                RecommendationDirection::Download,
                RecommendationDirection::Upload,
            ] {
                let current_rate = Self::site_rate(site, direction);
                let planned_rate = Self::planned_rate(&site.config, direction);
                if current_rate == planned_rate {
                    continue;
                }

                let interface_name = Self::interface_name(config, direction);
                info!(
                    "Replaying persisted StormGuard {} override for {}: {} -> {}",
                    direction, name, planned_rate, current_rate
                );
                Self::apply_dependents(
                    &site.config,
                    direction,
                    current_rate,
                    config,
                    &interface_name,
                    bakery_sender.clone(),
                );
                Self::apply_htb_change(
                    config,
                    &interface_name,
                    queue.class_id,
                    current_rate,
                    bakery_sender.clone(),
                );
            }
        }
    }

    pub async fn read_new_tick_data(
        &mut self,
        config: &StormguardConfig,
        active_ping_sample: Option<TimedRtt>,
        active_ping_updated: bool,
    ) {
        let requests = vec![BusRequest::GetFullNetworkMap];
        let Ok(responses) = lqos_bus::bus_request(requests).await else {
            info!("Failed to get lqosd stats");
            return;
        };

        for site in self.sites.values_mut() {
            site.current_throughput = (0.0, 0.0);
            site.clear_tick_rtt_state();
        }

        for response in responses {
            let BusResponse::NetworkMap(all_nodes) = response else {
                continue;
            };
            for (_, node_info) in all_nodes {
                let Some(target) = self.sites.get_mut(&node_info.name) else {
                    continue;
                };

                // Record throughput (Mbps)
                let down_mbps = (node_info.current_throughput.0 as f64 * 8.0) / 1_000_000.0;
                let up_mbps = (node_info.current_throughput.1 as f64 * 8.0) / 1_000_000.0;
                target.throughput_down.add(down_mbps);
                target.throughput_up.add(up_mbps);
                target.current_throughput = (down_mbps, up_mbps);

                // Retransmits (as a percentage of TCP packets)
                let retransmits_down = if node_info.current_tcp_packets.0 > 0 {
                    node_info.current_retransmits.0 as f64 / node_info.current_tcp_packets.0 as f64
                } else {
                    0.0
                };
                let retransmits_up = if node_info.current_tcp_packets.1 > 0 {
                    node_info.current_retransmits.1 as f64 / node_info.current_tcp_packets.1 as f64
                } else {
                    0.0
                };
                target.retransmits_down.add(retransmits_down);
                target.retransmits_up.add(retransmits_up);

                // Round-Trip Time
                if !node_info.rtts.is_empty() {
                    let mut my_round_trip_times = node_info.rtts.clone();
                    my_round_trip_times.sort_by(|a, b| a.total_cmp(b));
                    let samples = my_round_trip_times.len();
                    let mut idx = ((samples as f32) * 0.9).floor() as usize;
                    idx = idx.min(samples.saturating_sub(1));
                    let p90 = my_round_trip_times[idx] as f64;
                    target.record_passive_rtt_sample(p90);
                }
            }
        }

        let now = Instant::now();
        let passive_max_age = Duration::from_secs(15);
        let active_max_age = Duration::from_secs_f32(
            (config.active_ping_interval_seconds.max(1.0) * 3.0).clamp(5.0, 300.0),
        );
        let active_weight = config.active_ping_weight.clamp(0.0, 1.0) as f64;

        for site in self.sites.values_mut() {
            let passive = site.last_passive_rtt().and_then(|(ms, at)| {
                if now.duration_since(at) <= passive_max_age {
                    Some(ms)
                } else {
                    None
                }
            });
            let active = active_ping_sample.and_then(|s| {
                if now.duration_since(s.at) <= active_max_age {
                    Some(s.rtt_ms)
                } else {
                    None
                }
            });

            site.passive_rtt_ms = passive;
            site.active_ping_rtt_ms = active;

            let effective = if matches!(
                config.strategy,
                lqos_config::StormguardStrategy::DelayProbeActive
            ) {
                match (active, passive) {
                    (Some(active_ms), Some(passive_ms)) => {
                        Some(active_weight * active_ms + (1.0 - active_weight) * passive_ms)
                    }
                    (Some(active_ms), None) => Some(active_ms),
                    (None, Some(passive_ms)) => Some(passive_ms),
                    (None, None) => None,
                }
            } else {
                passive
            };

            site.current_rtt_ms = effective;

            let active_updated = matches!(
                config.strategy,
                lqos_config::StormguardStrategy::DelayProbeActive
            ) && active_ping_updated
                && active.is_some();
            let updated = site.passive_rtt_updated_this_tick() || active_updated;
            if updated && let Some(effective) = effective {
                site.round_trip_time.add(effective);
                site.rtt_sample_for_baseline_ms = Some(effective);
            }
        }
    }

    pub fn check_state(&mut self, config: &StormguardConfig) {
        self.sites
            .iter_mut()
            .for_each(|(_, s)| s.check_state(config));
    }

    pub fn recommendations(&mut self, config: &StormguardConfig) -> Vec<(Recommendation, String)> {
        let mut recommendations = Vec::new();
        self.sites
            .iter_mut()
            .for_each(|(_, s)| s.recommendations(&mut recommendations, config));
        recommendations
    }

    pub fn debug_snapshot(&self, config: &StormguardConfig) -> Vec<StormguardDebugEntry> {
        self.sites
            .iter()
            .filter_map(|(name, site)| {
                let site_config = config.sites.get(name)?;

                let state_string = |state: &StormguardState| -> (String, Option<f32>) {
                    match state {
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
                    }
                };

                let baseline_rtt_ms = site.rtt_baseline_ms;
                let rtt_now = site
                    .current_rtt_ms
                    .or_else(|| site.round_trip_time.average());
                let delay_ms = match (rtt_now, baseline_rtt_ms) {
                    (Some(rtt), Some(baseline)) => Some((rtt - baseline).max(0.0)),
                    _ => None,
                };
                let strategy = match config.strategy {
                    lqos_config::StormguardStrategy::LegacyScore => "legacy_score",
                    lqos_config::StormguardStrategy::DelayProbe => "delay_probe",
                    lqos_config::StormguardStrategy::DelayProbeActive => "delay_probe_active",
                };
                let rtt = site.round_trip_time.average();
                let rtt_ma = site.round_trip_time_moving_average.average();
                let action_string = |action: RecommendationAction| -> &'static str {
                    match action {
                        RecommendationAction::IncreaseFast => "increase_fast",
                        RecommendationAction::Increase => "increase",
                        RecommendationAction::Decrease => "decrease",
                        RecommendationAction::DecreaseFast => "decrease_fast",
                    }
                };

                let make_direction =
                    |direction: RecommendationDirection| -> StormguardDebugDirection {
                        let (
                            queue_mbps,
                            min_mbps,
                            max_mbps,
                            throughput_mbps,
                            throughput_ma_mbps,
                            retrans,
                            retrans_ma,
                        ) = match direction {
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
                            RecommendationDirection::Download => state_string(&site.download_state),
                            RecommendationDirection::Upload => state_string(&site.upload_state),
                        };

                        let (last_action, last_action_age_secs) = match direction {
                            RecommendationDirection::Download => site.last_action_download,
                            RecommendationDirection::Upload => site.last_action_upload,
                        }
                        .map(|(action, at)| {
                            (
                                Some(action_string(action).to_string()),
                                Some(at.elapsed().as_secs_f32()),
                            )
                        })
                        .unwrap_or((None, None));

                        let saturation_max =
                            SaturationLevel::from_throughput(throughput_mbps, max_mbps as f64);
                        let saturation_current =
                            SaturationLevel::from_throughput(throughput_mbps, queue_mbps as f64);

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
                            rtt,
                            rtt_ma,
                            passive_rtt_ms: site.passive_rtt_ms,
                            active_ping_rtt_ms: site.active_ping_rtt_ms,
                            baseline_rtt_ms,
                            delay_ms,
                            strategy: strategy.to_string(),
                            last_action,
                            last_action_age_secs,
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
        let mut pending_site_updates: HashSet<String> = HashSet::new();

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
            let cooldown_secs = Self::cooldown_for_action(config, &recommendation.action);
            debug!(
                "Cooldown for {:?} set to {:.1}s",
                recommendation.action, cooldown_secs
            );

            // Circuit queues host qdiscs; prefer the TreeGuard-style fallback path.
            if let Some(circuit_id) = queue.circuit_id.as_deref() {
                Self::handle_circuit_queue_recommendation(CircuitQueueRecommendationContext {
                    active_circuit_fallbacks: &mut self.active_circuit_fallbacks,
                    site,
                    config,
                    recommendation: &recommendation,
                    summary: &summary,
                    circuit_id,
                    cooldown_secs,
                    log_sender: &log_sender,
                    bakery_sender: bakery_sender.clone(),
                });
                continue;
            }

            let interface_name = Self::interface_name(config, recommendation.direction);

            // Find the TC class
            let class_handle = queue.class_id;

            // Find the new bandwidth
            let current_rate = Self::site_rate(site, recommendation.direction) as f64;
            let max_rate = Self::planned_rate(site_config, recommendation.direction) as f64;
            let min_rate = Self::minimum_rate(site_config, recommendation.direction) as f64;

            let new_rate_multiplier = Self::multiplier_for_action(config, &recommendation.action);
            let new_rate = current_rate * new_rate_multiplier;
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

            if config.dry_run {
                Self::apply_dependents(
                    &site.config,
                    recommendation.direction,
                    new_rate,
                    config,
                    &interface_name,
                    bakery_sender.clone(),
                );
                Self::apply_htb_change(
                    config,
                    &interface_name,
                    class_handle,
                    new_rate,
                    bakery_sender.clone(),
                );
                Self::enter_cooldown(
                    site,
                    recommendation.direction,
                    cooldown_secs,
                    recommendation.action,
                );
                let _ = log_sender.send(LogCommand::SpeedChange {
                    site: recommendation.site.clone(),
                    download: site.queue_download_mbps,
                    upload: site.queue_upload_mbps,
                    state: format!("{summary}; dry_run_target={new_rate}"),
                });
                continue;
            }

            // Apply to the site
            Self::set_site_rate(site, recommendation.direction, new_rate);

            // Actually make the change
            Self::apply_dependents(
                &site.config,
                recommendation.direction,
                new_rate,
                config,
                &interface_name,
                bakery_sender.clone(),
            );
            Self::apply_htb_change(
                config,
                &interface_name,
                class_handle,
                new_rate,
                bakery_sender.clone(),
            );
            pending_site_updates.insert(site.config.name.clone());

            // Finish Up by entering cooldown
            debug!("Recommendation applied: entering cooldown");
            Self::enter_cooldown(
                site,
                recommendation.direction,
                cooldown_secs,
                recommendation.action,
            );

            // Report
            let _ = log_sender.send(LogCommand::SpeedChange {
                site: recommendation.site.clone(),
                download: site.queue_download_mbps,
                upload: site.queue_upload_mbps,
                state: summary,
            });
        }

        if !pending_site_updates.is_empty() {
            let updates: Vec<SiteOverrideUpdate> = pending_site_updates
                .into_iter()
                .filter_map(|site_name| {
                    self.sites
                        .get(&site_name)
                        .map(Self::site_override_update_from_state)
                })
                .collect();
            if let Err(e) = apply_site_override_updates(&updates) {
                warn!("Failed to batch StormGuard site override updates: {}", e);
            }
        }
    }

    fn handle_circuit_queue_recommendation(ctx: CircuitQueueRecommendationContext<'_>) {
        let CircuitQueueRecommendationContext {
            active_circuit_fallbacks,
            site,
            config,
            recommendation,
            summary,
            circuit_id,
            cooldown_secs,
            log_sender,
            bakery_sender,
        } = ctx;
        let outcome = if !config.circuit_fallback_enabled {
            CircuitFallbackOutcome::Skipped {
                reason: "Circuit fallback disabled in config.".to_string(),
            }
        } else if matches!(
            recommendation.action,
            RecommendationAction::Increase | RecommendationAction::IncreaseFast
        ) && !active_circuit_fallbacks.contains(circuit_id)
        {
            CircuitFallbackOutcome::Skipped {
                reason: "No active StormGuard circuit fallback to clear.".to_string(),
            }
        } else {
            match recommendation.action {
                RecommendationAction::Decrease | RecommendationAction::DecreaseFast => {
                    match apply_circuit_fallback(
                        circuit_id,
                        &config.circuit_fallback_sqm,
                        config.circuit_fallback_persist,
                        config.dry_run,
                        bakery_sender,
                    ) {
                        Ok(outcome) => outcome,
                        Err(e) => {
                            warn!(
                                "StormGuard fallback failed for circuit {} ({}): {}",
                                circuit_id, recommendation.site, e
                            );
                            CircuitFallbackOutcome::Skipped {
                                reason: format!("Fallback error: {e}"),
                            }
                        }
                    }
                }
                RecommendationAction::Increase | RecommendationAction::IncreaseFast => {
                    match clear_circuit_fallback(circuit_id, config.dry_run, bakery_sender) {
                        Ok(outcome) => outcome,
                        Err(e) => {
                            warn!(
                                "StormGuard fallback clear failed for circuit {} ({}): {}",
                                circuit_id, recommendation.site, e
                            );
                            CircuitFallbackOutcome::Skipped {
                                reason: format!("Fallback clear error: {e}"),
                            }
                        }
                    }
                }
            }
        };

        let (outcome_text, enters_cooldown) = match outcome {
            CircuitFallbackOutcome::Applied { persisted } => {
                active_circuit_fallbacks.insert(circuit_id.to_string());
                info!(
                    "StormGuard applied circuit fallback for {} ({})",
                    recommendation.site, circuit_id
                );
                (
                    format!(
                        "circuit_fallback=applied sqm={} persisted={persisted}",
                        config.circuit_fallback_sqm
                    ),
                    true,
                )
            }
            CircuitFallbackOutcome::Cleared { persisted } => {
                active_circuit_fallbacks.remove(circuit_id);
                info!(
                    "StormGuard cleared circuit fallback for {} ({})",
                    recommendation.site, circuit_id
                );
                (
                    format!("circuit_fallback=cleared persisted={persisted}"),
                    true,
                )
            }
            CircuitFallbackOutcome::DryRun { action } => {
                info!(
                    "StormGuard dry-run circuit fallback for {} ({}): {}",
                    recommendation.site, circuit_id, action
                );
                (format!("circuit_fallback=dry_run {action}"), true)
            }
            CircuitFallbackOutcome::Skipped { reason } => {
                warn!(
                    "StormGuard skipped circuit fallback for {} ({}): {}",
                    recommendation.site, circuit_id, reason
                );
                (format!("circuit_fallback=skipped reason={reason}"), false)
            }
        };

        if Self::circuit_outcome_enters_cooldown(&enters_cooldown) {
            Self::enter_cooldown(
                site,
                recommendation.direction,
                cooldown_secs,
                recommendation.action,
            );
        }
        let _ = log_sender.send(LogCommand::SpeedChange {
            site: recommendation.site.clone(),
            download: site.queue_download_mbps,
            upload: site.queue_upload_mbps,
            state: format!("{summary}; {outcome_text}"),
        });
    }

    fn site_rate(site: &SiteState, direction: RecommendationDirection) -> u64 {
        match direction {
            RecommendationDirection::Download => site.queue_download_mbps,
            RecommendationDirection::Upload => site.queue_upload_mbps,
        }
    }

    fn planned_rate(site: &crate::config::WatchingSite, direction: RecommendationDirection) -> u64 {
        match direction {
            RecommendationDirection::Download => site.max_download_mbps,
            RecommendationDirection::Upload => site.max_upload_mbps,
        }
    }

    fn minimum_rate(site: &crate::config::WatchingSite, direction: RecommendationDirection) -> u64 {
        match direction {
            RecommendationDirection::Download => site.min_download_mbps,
            RecommendationDirection::Upload => site.min_upload_mbps,
        }
    }

    fn interface_name(config: &StormguardConfig, direction: RecommendationDirection) -> String {
        match direction {
            RecommendationDirection::Download => config.download_interface.clone(),
            RecommendationDirection::Upload => config.upload_interface.clone(),
        }
    }

    fn multiplier_for_action(config: &StormguardConfig, action: &RecommendationAction) -> f64 {
        match action {
            RecommendationAction::IncreaseFast => config.increase_fast_multiplier,
            RecommendationAction::Increase => config.increase_multiplier,
            RecommendationAction::Decrease => config.decrease_multiplier,
            RecommendationAction::DecreaseFast => config.decrease_fast_multiplier,
        }
    }

    fn cooldown_for_action(config: &StormguardConfig, action: &RecommendationAction) -> f32 {
        match action {
            RecommendationAction::IncreaseFast => config.increase_fast_cooldown_seconds,
            RecommendationAction::Increase => config.increase_cooldown_seconds,
            RecommendationAction::Decrease => config.decrease_cooldown_seconds,
            RecommendationAction::DecreaseFast => config.decrease_fast_cooldown_seconds,
        }
    }

    fn set_site_rate(site: &mut SiteState, direction: RecommendationDirection, new_rate: u64) {
        match direction {
            RecommendationDirection::Download => {
                site.queue_download_mbps = new_rate;
                site.ticks_since_last_probe_download = 0;
                let mut lock = crate::STORMGUARD_STATS.lock();
                if let Some(entry) = lock.iter_mut().find(|(n, _, _)| n == &site.config.name) {
                    entry.1 = new_rate;
                }
            }
            RecommendationDirection::Upload => {
                site.queue_upload_mbps = new_rate;
                site.ticks_since_last_probe_upload = 0;
                let mut lock = crate::STORMGUARD_STATS.lock();
                if let Some(entry) = lock.iter_mut().find(|(n, _, _)| n == &site.config.name) {
                    entry.2 = new_rate;
                }
            }
        }
    }

    fn apply_dependents(
        site: &crate::config::WatchingSite,
        direction: RecommendationDirection,
        new_rate: u64,
        config: &StormguardConfig,
        interface_name: &str,
        bakery_sender: Sender<BakeryCommands>,
    ) {
        for dependent in &site.dependent_nodes {
            let max_rate = match direction {
                RecommendationDirection::Download => dependent.original_max_download_mbps,
                RecommendationDirection::Upload => dependent.original_max_upload_mbps,
            };
            if max_rate < new_rate {
                continue;
            }
            info!(
                "Applying rate change to dependent {}: {} -> {}",
                dependent.name, max_rate, new_rate
            );
            Self::apply_htb_change(
                config,
                interface_name,
                dependent.class_id,
                new_rate,
                bakery_sender.clone(),
            );
        }
    }

    fn site_override_update_from_state(site: &SiteState) -> SiteOverrideUpdate {
        SiteOverrideUpdate {
            site_name: site.config.name.clone(),
            download_bandwidth_mbps: (site.queue_download_mbps != site.config.max_download_mbps)
                .then_some(site.queue_download_mbps as u32),
            upload_bandwidth_mbps: (site.queue_upload_mbps != site.config.max_upload_mbps)
                .then_some(site.queue_upload_mbps as u32),
        }
    }

    fn circuit_outcome_enters_cooldown(enters_cooldown: &bool) -> bool {
        *enters_cooldown
    }

    fn enter_cooldown(
        site: &mut SiteState,
        direction: RecommendationDirection,
        cooldown_secs: f32,
        action: RecommendationAction,
    ) {
        let now = Instant::now();
        match direction {
            RecommendationDirection::Download => {
                site.download_state = StormguardState::Cooldown {
                    start: now,
                    duration_secs: cooldown_secs,
                };
                site.ticks_since_last_probe_download = 0;
                site.last_action_download = Some((action, now));
            }
            RecommendationDirection::Upload => {
                site.upload_state = StormguardState::Cooldown {
                    start: now,
                    duration_secs: cooldown_secs,
                };
                site.ticks_since_last_probe_upload = 0;
                site.last_action_upload = Some((action, now));
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StormguardConfig as RuntimeStormguardConfig;
    use crate::config::WatchingSite;
    use lqos_config::StormguardStrategy;
    use std::collections::HashMap;

    fn site_state(download: u64, upload: u64, max_down: u64, max_up: u64) -> SiteState {
        SiteState {
            config: WatchingSite {
                name: "Site A".to_string(),
                max_download_mbps: max_down,
                max_upload_mbps: max_up,
                min_download_mbps: 10,
                min_upload_mbps: 10,
                dependent_nodes: Vec::new(),
                current_download_mbps: download,
                current_upload_mbps: upload,
            },
            download_state: StormguardState::Running,
            upload_state: StormguardState::Running,
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
            queue_download_mbps: download,
            queue_upload_mbps: upload,
            current_throughput: (0.0, 0.0),
            current_rtt_ms: None,
            passive_rtt_ms: None,
            active_ping_rtt_ms: None,
            rtt_sample_for_baseline_ms: None,
            rtt_baseline_ms: None,
            last_passive_rtt_ms: None,
            last_passive_rtt_at: None,
            passive_rtt_updated_this_tick: false,
            last_action_download: None,
            last_action_upload: None,
            ticks_since_last_probe_download: 0,
            ticks_since_last_probe_upload: 0,
        }
    }

    fn test_config(strategy: StormguardStrategy) -> RuntimeStormguardConfig {
        RuntimeStormguardConfig {
            sites: HashMap::new(),
            download_interface: "eth0".to_string(),
            upload_interface: "eth1".to_string(),
            dry_run: true,
            log_filename: None,
            strategy,
            increase_fast_multiplier: 1.30,
            increase_multiplier: 1.15,
            decrease_multiplier: 0.95,
            decrease_fast_multiplier: 0.88,
            increase_fast_cooldown_seconds: 2.0,
            increase_cooldown_seconds: 1.0,
            decrease_cooldown_seconds: 3.75,
            decrease_fast_cooldown_seconds: 7.5,
            circuit_fallback_enabled: false,
            circuit_fallback_persist: true,
            circuit_fallback_sqm: "fq_codel".to_string(),
            delay_threshold_ms: 40.0,
            delay_threshold_ratio: 1.10,
            baseline_alpha_up: 0.01,
            baseline_alpha_down: 0.10,
            probe_interval_seconds: 10.0,
            min_throughput_mbps_for_rtt: 0.05,
            active_ping_target: "1.1.1.1".to_string(),
            active_ping_interval_seconds: 10.0,
            active_ping_weight: 0.70,
            active_ping_timeout_seconds: 1.0,
        }
    }

    #[test]
    fn site_override_update_omits_baseline_rates() {
        let site = site_state(100, 50, 100, 50);
        let update = SiteStateTracker::site_override_update_from_state(&site);
        assert_eq!(update.download_bandwidth_mbps, None);
        assert_eq!(update.upload_bandwidth_mbps, None);
    }

    #[test]
    fn site_override_update_keeps_only_changed_directions() {
        let site = site_state(75, 50, 100, 50);
        let update = SiteStateTracker::site_override_update_from_state(&site);
        assert_eq!(update.download_bandwidth_mbps, Some(75));
        assert_eq!(update.upload_bandwidth_mbps, None);
    }

    #[test]
    fn skipped_circuit_outcomes_do_not_enter_cooldown() {
        assert!(!SiteStateTracker::circuit_outcome_enters_cooldown(&false));
        assert!(SiteStateTracker::circuit_outcome_enters_cooldown(&true));
    }

    #[test]
    fn warmup_progresses_with_zero_samples() {
        let cfg = test_config(StormguardStrategy::LegacyScore);
        let mut site = site_state(50, 50, 100, 100);
        site.download_state = StormguardState::Warmup;
        site.upload_state = StormguardState::Warmup;

        for _ in 0..11 {
            site.throughput_down.add(0.0);
            site.throughput_up.add(0.0);
            site.retransmits_down.add(0.0);
            site.retransmits_up.add(0.0);
        }

        site.check_state(&cfg);
        assert_eq!(site.download_state, StormguardState::Running);
        assert_eq!(site.upload_state, StormguardState::Running);
    }

    #[test]
    fn delay_probe_decreases_on_bufferbloat() {
        let cfg = test_config(StormguardStrategy::DelayProbe);
        let mut site = site_state(20, 20, 50, 50);
        site.current_throughput = (10.0, 0.0);
        site.current_rtt_ms = Some(800.0);
        site.rtt_baseline_ms = Some(600.0);

        let mut recs = Vec::new();
        site.recommendations(&mut recs, &cfg);
        assert!(recs.iter().any(|(r, _)| {
            r.direction == RecommendationDirection::Download
                && matches!(
                    r.action,
                    RecommendationAction::DecreaseFast | RecommendationAction::Decrease
                )
        }));
    }

    #[test]
    fn delay_probe_increases_when_good_and_loaded() {
        let cfg = test_config(StormguardStrategy::DelayProbe);
        let mut site = site_state(10, 10, 50, 50);
        site.current_throughput = (9.0, 0.0);
        site.current_rtt_ms = Some(610.0);
        site.rtt_baseline_ms = Some(600.0);
        site.ticks_since_last_probe_download = 10;

        let mut recs = Vec::new();
        site.recommendations(&mut recs, &cfg);
        assert!(recs.iter().any(|(r, _)| {
            r.direction == RecommendationDirection::Download
                && matches!(
                    r.action,
                    RecommendationAction::Increase | RecommendationAction::IncreaseFast
                )
        }));
    }

    #[test]
    fn delay_probe_applies_rtt_logic_to_upload() {
        let cfg = test_config(StormguardStrategy::DelayProbe);
        let mut site = site_state(20, 20, 50, 50);
        site.current_throughput = (0.0, 18.0);
        site.current_rtt_ms = Some(900.0);
        site.rtt_baseline_ms = Some(600.0);

        let mut recs = Vec::new();
        site.recommendations(&mut recs, &cfg);
        assert!(recs.iter().any(|(r, _)| {
            r.direction == RecommendationDirection::Upload
                && matches!(
                    r.action,
                    RecommendationAction::DecreaseFast | RecommendationAction::Decrease
                )
        }));
    }
}
