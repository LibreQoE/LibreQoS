mod analysis;
mod recommendation;
mod ring_buffer;
mod site;
mod stormguard_state;

use crate::config::StormguardConfig;
use crate::datalog::LogCommand;
use crate::adaptive_actions::{
    CircuitFallbackOutcome, SiteOverrideUpdate, apply_circuit_fallback,
    apply_site_override_updates, clear_circuit_fallback, load_persisted_circuit_fallbacks,
};
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
use tracing::{debug, info, warn};

pub struct SiteStateTracker {
    sites: HashMap<String, SiteState>,
    active_circuit_fallbacks: HashSet<String>,
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
                    queue_download_mbps: site.current_download_mbps,
                    queue_upload_mbps: site.current_upload_mbps,
                    current_throughput: (0.0, 0.0),
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
            Err(e) => warn!("Failed to load persisted StormGuard circuit fallbacks: {}", e),
        }

        for (name, site) in &self.sites {
            let Some(queue) = queues.iter().find(|n| n.name.as_deref() == Some(name.as_str())) else {
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
                                RecommendationDirection::Upload => {
                                    site_config.max_upload_mbps as f64
                                }
                            },
                        );
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
                Self::handle_circuit_queue_recommendation(
                    &mut self.active_circuit_fallbacks,
                    site,
                    config,
                    &recommendation,
                    &summary,
                    circuit_id,
                    cooldown_secs,
                    &log_sender,
                    bakery_sender.clone(),
                );
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
                Self::enter_cooldown(site, recommendation.direction, cooldown_secs);
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
            Self::enter_cooldown(site, recommendation.direction, cooldown_secs);

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

    fn handle_circuit_queue_recommendation(
        active_circuit_fallbacks: &mut HashSet<String>,
        site: &mut SiteState,
        config: &StormguardConfig,
        recommendation: &Recommendation,
        summary: &str,
        circuit_id: &str,
        cooldown_secs: f32,
        log_sender: &std::sync::mpsc::Sender<LogCommand>,
        bakery_sender: Sender<BakeryCommands>,
    ) {
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
                (format!("circuit_fallback=cleared persisted={persisted}"), true)
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
            Self::enter_cooldown(site, recommendation.direction, cooldown_secs);
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
                    if let Some(entry) =
                        lock.iter_mut().find(|(n, _, _)| n == &site.config.name)
                    {
                        entry.1 = new_rate;
                    }
                }
                RecommendationDirection::Upload => {
                    site.queue_upload_mbps = new_rate;
                    site.ticks_since_last_probe_upload = 0;
                    let mut lock = crate::STORMGUARD_STATS.lock();
                    if let Some(entry) =
                        lock.iter_mut().find(|(n, _, _)| n == &site.config.name)
                    {
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

    fn enter_cooldown(site: &mut SiteState, direction: RecommendationDirection, cooldown_secs: f32) {
        match direction {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WatchingSite;

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
            ticks_since_last_probe_download: 0,
            ticks_since_last_probe_upload: 0,
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
}
