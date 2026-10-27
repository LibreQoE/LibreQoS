mod analysis;
mod recommendation;
mod ring_buffer;
mod site;
mod stormguard_state;

use crate::active_ping::TimedRtt;
use crate::bus::StormguardBusSender;
use crate::adaptive_actions::{
    CircuitFallbackOutcome, SiteOverrideUpdate, apply_circuit_fallback_layer,
    apply_site_override_updates, clear_circuit_fallback_layer, clear_stormguard_override_layer,
    operator_owns_site_speed,
};
use crate::config::StormguardConfig;
use crate::site_state::analysis::SaturationLevel;
use crate::site_state::recommendation::{
    Recommendation, RecommendationAction, RecommendationDirection,
};
use crate::site_state::ring_buffer::RingBuffer;
use crate::site_state::site::{ActionAttempt, DirectionDecision, SiteState};
use crate::site_state::stormguard_state::StormguardState;
use crate::{MOVING_AVERAGE_BUFFER_SIZE, READING_ACCUMULATOR_SIZE};
use anyhow::{Context, anyhow};
use lqos_bus::{BridgePublishStatus, StormguardDebugDirection, StormguardDebugEntry};
use lqos_config::NetworkJsonTransport;
use lqos_overrides::{OverrideFile, OverrideLayer, OverrideStore};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

pub struct SiteStateTracker {
    sites: HashMap<String, SiteState>,
    active_circuit_fallbacks: HashSet<String>,
    /// Sites already warned about an operator-owned refusal, so the loud WARN
    /// fires once per episode rather than every tick (K1b drill log-storm nit).
    operator_refusal_warned: HashSet<String>,
}

struct CircuitQueueRecommendationContext<'a> {
    active_circuit_fallbacks: &'a mut HashSet<String>,
    site: &'a mut SiteState,
    config: &'a StormguardConfig,
    recommendation: &'a Recommendation,
    circuit_id: &'a str,
    cooldown_secs: f32,
    bus_tx: StormguardBusSender,
    ttl_seconds: u64,
    publish_wait_seconds: u64,
}

#[derive(Default)]
pub(crate) struct ApplicationReport {
    pub(crate) errors: Vec<String>,
    pub(crate) acknowledged_success: bool,
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn unix_time_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn successful_application_outcome(dry_run: bool) -> &'static str {
    if dry_run { "dry_run" } else { "applied" }
}

fn active_rtt_source(
    passive_rtt: Option<f64>,
    active_rtt: Option<f64>,
    weight: f32,
) -> &'static str {
    match (passive_rtt, active_rtt) {
        (Some(_), Some(_)) if weight <= 0.0 => "passive",
        (Some(_), Some(_)) if weight >= 1.0 => "active",
        (Some(_), Some(_)) => "blended",
        (None, Some(_)) => "active",
        (Some(_), None) => "passive",
        (None, None) => "none",
    }
}

pub async fn reconcile_inactive_state() -> anyhow::Result<()> {
    // K1b startup decoupling §1.3: the kernel class-handle restorations ran only
    // where kernels attached; a bridge-only box restores via the fold after the
    // layer clear below (no queue structure needed, no WARN storm).
    clear_stormguard_override_layer().context("failed to clear inactive StormGuard overrides")?;
    Ok(())
}

pub(crate) fn restore_override_snapshot(snapshot: &OverrideFile, context: &str) -> anyhow::Result<()> {
    // K1b §4.6 pin 1: a snapshot restore must not resurrect a clamp whose TTL
    // lapsed while the restore was being prepared.
    let mut filtered = snapshot.clone();
    filtered.remove_lapsed_site_speed_overrides(unix_time_secs());
    OverrideStore::save_layer(OverrideLayer::Stormguard, &filtered)
        .with_context(|| format!("failed to restore StormGuard ownership {context}"))
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
                    passive_rtt_flow_counts: (0, 0),
                    rtt_sample_for_baseline_ms: None,
                    rtt_baseline_ms: None,
                    last_passive_rtt_ms: None,
                    last_passive_rtt_at: None,
                    passive_rtt_updated_this_tick: false,
                    last_action_download: None,
                    last_action_upload: None,
                    download_decision: DirectionDecision::default(),
                    upload_decision: DirectionDecision::default(),
                    last_attempt_download: None,
                    last_attempt_upload: None,
                    ticks_since_last_probe_download: 0,
                    ticks_since_last_probe_upload: 0,
                },
            );
        }
        Self {
            sites,
            active_circuit_fallbacks: HashSet::new(),
            operator_refusal_warned: HashSet::new(),
        }
    }

    pub async fn reset_adjustments(&mut self) -> anyhow::Result<()> {
        // K1b startup decoupling §1.3: the kernel class restorations + circuit
        // fallback clears ran only where kernels attached; the layer clear below
        // is the bridge-path restore (base rates return by construction).
        clear_stormguard_override_layer()
            .context("failed to clear persisted StormGuard adjustments")?;
        self.active_circuit_fallbacks.clear();
        Ok(())
    }

    pub fn read_new_tick_data(
        &mut self,
        config: &StormguardConfig,
        active_ping_sample: Option<TimedRtt>,
        active_ping_updated: bool,
        all_nodes: Vec<(usize, NetworkJsonTransport)>,
        // K2 slice 2 729b: bridged per-node passive RTT p90 (ms), keyed by
        // node name. `None` = kernel path (the rtts p50-based p90 stands).
        bridge_rtt_p90_ms: Option<&std::collections::HashMap<String, f32>>,
    ) {
        for site in self.sites.values_mut() {
            site.current_throughput = (0.0, 0.0);
            site.clear_tick_rtt_state();
        }

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
            target.passive_rtt_flow_counts = node_info.current_rtt_flows;

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

            // Round-Trip Time. K2 slice 2 729b source dispatch: on a bridge
            // box the passive p90 comes from the bridge's RTT histogram
            // (TreeLiveView); otherwise the per-node `rtts` p50-based p90
            // (kernel path / bridge-inactive honest fallback).
            if let Some(p90) = bridge_rtt_p90_ms.and_then(|map| map.get(&node_info.name).copied())
            {
                target.record_passive_rtt_sample(p90 as f64);
            } else if !node_info.rtts.is_empty() {
                let mut my_round_trip_times = node_info.rtts.clone();
                my_round_trip_times.sort_by(|a, b| a.total_cmp(b));
                let samples = my_round_trip_times.len();
                let mut idx = ((samples as f32) * 0.9).floor() as usize;
                idx = idx.min(samples.saturating_sub(1));
                let p90 = my_round_trip_times[idx] as f64;
                target.record_passive_rtt_sample(p90);
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
                        let decision = site.decision(direction);
                        let attempt = site.last_attempt(direction);

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
                            decision_score: decision.score,
                            candidate_action: decision
                                .candidate_action
                                .map(action_string)
                                .map(str::to_string),
                            candidate_target_mbps: decision.target_mbps,
                            decision_reason: decision.reason.clone(),
                            decision_blocker: decision.blocker.clone(),
                            last_attempt_action: attempt
                                .map(|attempt| action_string(attempt.action).to_string()),
                            last_attempt_target_mbps: attempt.map(|attempt| attempt.target_mbps),
                            last_attempt_outcome: attempt.map(|attempt| attempt.outcome.clone()),
                            last_attempt_unix_ms: attempt.map(|attempt| attempt.unix_ms),
                            last_attempt_error: attempt.and_then(|attempt| attempt.error.clone()),
                            rtt_source: if matches!(
                                config.strategy,
                                lqos_config::StormguardStrategy::DelayProbeActive
                            ) {
                                active_rtt_source(
                                    site.passive_rtt_ms,
                                    site.active_ping_rtt_ms,
                                    config.active_ping_weight,
                                )
                            } else if site.passive_rtt_ms.is_some() {
                                "passive"
                            } else {
                                "none"
                            }
                            .to_string(),
                        }
                    };

                Some(StormguardDebugEntry {
                    site: name.clone(),
                    download: make_direction(RecommendationDirection::Download),
                    upload: make_direction(RecommendationDirection::Upload),
                    active_ping_target: config.active_ping_target.clone(),
                    active_ping_weight: config.active_ping_weight,
                    passive_rtt_flow_counts: site.passive_rtt_flow_counts,
                })
            })
            .collect()
    }

    pub async fn apply_recommendations(
        &mut self,
        recommendations: Vec<(Recommendation, String)>,
        config: &StormguardConfig,
        bus_tx: StormguardBusSender,
    ) -> ApplicationReport {
        let mut application_errors = Vec::new();
        let mut acknowledged_success = false;
        // K1b §4.3: site clamps are batched into ONE override-layer write (=
        // one mint = one transaction) after the per-recommendation loop.
        let mut pending_clamps: Vec<(
            String,
            RecommendationDirection,
            RecommendationAction,
            u64,
            f32,
        )> = Vec::new();
        // K1b startup decoupling §1.3: site-level recommendations need only the
        // watched-site config + the override-layer machinery. The queue
        // structure is required only for the kernel/phase-2 circuit branch.
        // K3-B: the kernel queue structure is retired; there is no
        // kernel-resolved circuit id on the bridge path.
        let circuit_id: Option<String> = None;

        for (recommendation, _summary) in recommendations {
            // Find the Site Object
            let Some(site) = self.sites.get_mut(&recommendation.site) else {
                continue;
            };
            // Find the Site Config
            let Some(site_config) = config.sites.get(&recommendation.site) else {
                Self::record_skipped_application(
                    site,
                    &recommendation,
                    "watched-site configuration unavailable",
                );
                application_errors.push(format!(
                    "StormGuard watched-site configuration is unavailable for {}",
                    recommendation.site
                ));
                continue;
            };

            let cooldown_secs = Self::cooldown_for_action(config, &recommendation.action);
            debug!(
                "Cooldown for {:?} set to {:.1}s",
                recommendation.action, cooldown_secs
            );

            // Circuit queues host qdiscs; the LayerApply fallback path used the
            // kernel queue structure to resolve the circuit id. K3-B retires
            // that source; the site-level clamp path below is the bridge path.
            if let Some(circuit_id) = circuit_id.clone() {
                // K1b ph2 §9.3: the LayerApply strategy (layer entry + publish)
                // is the only circuit-fallback path now that the kernel is gone.
                if let Some(error) =
                    Self::handle_circuit_queue_recommendation(CircuitQueueRecommendationContext {
                        active_circuit_fallbacks: &mut self.active_circuit_fallbacks,
                        site,
                        config,
                        recommendation: &recommendation,
                        circuit_id: &circuit_id,
                        cooldown_secs,
                        bus_tx: bus_tx.clone(),
                        ttl_seconds: config.override_ttl_seconds,
                        publish_wait_seconds: config.publish_wait_seconds,
                    })
                    .await
                {
                    application_errors.push(error);
                }
                if site
                    .last_attempt(recommendation.direction)
                    .is_some_and(|attempt| {
                        matches!(attempt.outcome.as_str(), "applied" | "dry_run")
                    })
                {
                    acknowledged_success = true;
                }
                continue;
            }

            // Find the new bandwidth
            let Some(new_rate) = site.decision(recommendation.direction).target_mbps else {
                Self::record_skipped_application(
                    site,
                    &recommendation,
                    "decision has no applicable target rate",
                );
                continue;
            };
            let min_rate = Self::minimum_rate(site_config, recommendation.direction);
            let max_rate = Self::planned_rate(site_config, recommendation.direction);
            if !(min_rate..=max_rate).contains(&new_rate) {
                Self::record_skipped_application(
                    site,
                    &recommendation,
                    "decision target is outside the configured bounds",
                );
                continue;
            }

            // K1b §4.2 operator supremacy: refuse up-front to clamp an
            // operator-owned site (loud, never a silent no-op drop).
            match operator_owns_site_speed(site.config.node_id.as_deref(), &recommendation.site) {
                Ok(true) => {
                    let reason =
                        "Operator site-speed override is present; StormGuard will not clamp it";
                    if self.operator_refusal_warned.insert(recommendation.site.clone()) {
                        warn!(
                            "StormGuard refusing to clamp {}: {reason}",
                            recommendation.site
                        );
                    }
                    Self::record_skipped_application(site, &recommendation, reason);
                    continue;
                }
                Ok(false) => {
                    // Re-arm the one-shot WARN once the operator entry clears.
                    self.operator_refusal_warned.remove(&recommendation.site);
                }
                Err(error) => {
                    let error = format!(
                        "could not read operator overrides before clamping {}: {error}",
                        recommendation.site
                    );
                    warn!("{error}");
                    application_errors.push(error);
                    continue;
                }
            }

            if config.dry_run {
                Self::complete_successful_application(
                    site,
                    recommendation.direction,
                    recommendation.action,
                    new_rate,
                    cooldown_secs,
                    true,
                );
                acknowledged_success = true;
                continue;
            }

            pending_clamps.push((
                recommendation.site.clone(),
                recommendation.direction,
                recommendation.action,
                new_rate,
                cooldown_secs,
            ));
        }

        if !pending_clamps.is_empty() {
            // Merge per-site updates so a pass that clamps both directions (or
            // restores one while clamping the other) writes ONE coherent entry
            // per site, matching the old immediate-apply semantics.
            let mut merged: HashMap<String, SiteOverrideUpdate> = HashMap::new();
            for (site_name, direction, _, target, _) in &pending_clamps {
                let Some(site) = self.sites.get(site_name) else {
                    continue;
                };
                let update = Self::site_override_update_with_target(site, *direction, *target);
                let entry = merged
                    .entry(site_name.clone())
                    .or_insert_with(|| Self::site_override_update_from_state(site));
                match direction {
                    RecommendationDirection::Download => {
                        entry.download_bandwidth_mbps = update.download_bandwidth_mbps;
                    }
                    RecommendationDirection::Upload => {
                        entry.upload_bandwidth_mbps = update.upload_bandwidth_mbps;
                    }
                }
            }
            let updates: Vec<SiteOverrideUpdate> = merged.into_values().collect();
            let snapshot = OverrideStore::load_layer(OverrideLayer::Stormguard).ok();
            match apply_site_override_updates(&updates, config.override_ttl_seconds) {
                Ok(_) => {
                    if let Err(error) =
                        publish_clamp_and_wait(&bus_tx, config.publish_wait_seconds).await
                    {
                        // K1b §4.6 pin 2: on REJECTED (or a timed-out/failed
                        // wait) restore the pre-write snapshot so the bridge
                        // never keeps an unconfirmed clamp.
                        if let Some(snapshot) = &snapshot
                            && let Err(restore_error) =
                                restore_override_snapshot(snapshot, "after a rejected StormGuard clamp")
                        {
                            warn!("StormGuard could not restore overrides after a rejected clamp: {restore_error}");
                        }
                        let error = error.to_string();
                        warn!("StormGuard clamp publish failed: {error}");
                        for (site_name, direction, action, target, _) in &pending_clamps {
                            if let Some(site) = self.sites.get_mut(site_name) {
                                Self::record_failed_application(
                                    site,
                                    *direction,
                                    *action,
                                    *target,
                                    error.clone(),
                                );
                            }
                        }
                        application_errors.push(error);
                    } else {
                        for (site_name, direction, action, target, cooldown) in &pending_clamps {
                            if let Some(site) = self.sites.get_mut(site_name) {
                                Self::complete_successful_application(
                                    site,
                                    *direction,
                                    *action,
                                    *target,
                                    *cooldown,
                                    false,
                                );
                            }
                        }
                        acknowledged_success = true;
                    }
                }
                Err(error) => {
                    let error = error.to_string();
                    warn!("StormGuard clamp layer write failed: {error}");
                    for (site_name, direction, action, target, _) in &pending_clamps {
                        if let Some(site) = self.sites.get_mut(site_name) {
                            Self::record_failed_application(
                                site,
                                *direction,
                                *action,
                                *target,
                                error.clone(),
                            );
                        }
                    }
                    application_errors.push(error);
                }
            }
        }

        ApplicationReport {
            errors: application_errors,
            acknowledged_success,
        }
    }

    async fn handle_circuit_queue_recommendation(
        ctx: CircuitQueueRecommendationContext<'_>,
    ) -> Option<String> {
        let CircuitQueueRecommendationContext {
            active_circuit_fallbacks,
            site,
            config,
            recommendation,
            circuit_id,
            cooldown_secs,
            bus_tx,
            ttl_seconds,
            publish_wait_seconds,
        } = ctx;
        let outcome = if !config.circuit_fallback_enabled {
            Ok(CircuitFallbackOutcome::Skipped {
                reason: "Circuit fallback disabled in config.".to_string(),
            })
        } else if matches!(
            recommendation.action,
            RecommendationAction::Increase | RecommendationAction::IncreaseFast
        ) && !active_circuit_fallbacks.contains(circuit_id)
        {
            Ok(CircuitFallbackOutcome::Skipped {
                reason: "No active StormGuard circuit fallback to clear.".to_string(),
            })
        } else {
            // K1b ph2 §9.3: LayerApply (layer entry + publish) is the only
            // circuit-fallback path now that the kernel is gone.
            match recommendation.action {
                RecommendationAction::Decrease | RecommendationAction::DecreaseFast => {
                    apply_circuit_fallback_layer(
                        circuit_id,
                        &config.circuit_fallback_sqm,
                        config.dry_run,
                        ttl_seconds,
                        publish_wait_seconds,
                        bus_tx,
                    )
                    .await
                    .map_err(|error| format!("Fallback error: {error}"))
                }
                RecommendationAction::Increase | RecommendationAction::IncreaseFast => {
                    clear_circuit_fallback_layer(
                        circuit_id,
                        config.dry_run,
                        publish_wait_seconds,
                        bus_tx,
                    )
                    .await
                    .map_err(|error| format!("Fallback clear error: {error}"))
                }
            }
        };

        let (outcome_text, enters_cooldown, outcome_label, outcome_error) = match outcome {
            Ok(CircuitFallbackOutcome::Applied { persisted }) => {
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
                    "applied",
                    None,
                )
            }
            Ok(CircuitFallbackOutcome::Cleared { persisted }) => {
                active_circuit_fallbacks.remove(circuit_id);
                info!(
                    "StormGuard cleared circuit fallback for {} ({})",
                    recommendation.site, circuit_id
                );
                (
                    format!("circuit_fallback=cleared persisted={persisted}"),
                    true,
                    "applied",
                    None,
                )
            }
            Ok(CircuitFallbackOutcome::DryRun { action }) => {
                info!(
                    "StormGuard dry-run circuit fallback for {} ({}): {}",
                    recommendation.site, circuit_id, action
                );
                (
                    format!("circuit_fallback=dry_run {action}"),
                    true,
                    "dry_run",
                    None,
                )
            }
            Ok(CircuitFallbackOutcome::Skipped { reason }) => {
                warn!(
                    "StormGuard skipped circuit fallback for {} ({}): {}",
                    recommendation.site, circuit_id, reason
                );
                (
                    format!("circuit_fallback=skipped reason={reason}"),
                    false,
                    "skipped",
                    Some(reason),
                )
            }
            Err(reason) => {
                warn!(
                    "StormGuard circuit fallback failed for {} ({}): {}",
                    recommendation.site, circuit_id, reason
                );
                (
                    format!("circuit_fallback=failed reason={reason}"),
                    false,
                    "failed",
                    Some(reason),
                )
            }
        };

        let attempted_target = Self::site_rate(site, recommendation.direction);
        site.record_attempt(
            recommendation.direction,
            ActionAttempt {
                action: recommendation.action,
                target_mbps: attempted_target,
                outcome: outcome_label.to_string(),
                unix_ms: unix_time_ms(),
                error: outcome_error,
            },
        );

        if enters_cooldown {
            Self::enter_cooldown(
                site,
                recommendation.direction,
                cooldown_secs,
                recommendation.action,
            );
        }
        debug!("StormGuard circuit fallback outcome: {outcome_text}");
        (outcome_label == "failed").then_some(outcome_text)
    }

    fn site_rate(site: &SiteState, direction: RecommendationDirection) -> u64 {
        match direction {
            RecommendationDirection::Download => site.queue_download_mbps,
            RecommendationDirection::Upload => site.queue_upload_mbps,
        }
    }

    fn attempt_target(site: &SiteState, direction: RecommendationDirection) -> u64 {
        site.decision(direction)
            .target_mbps
            .unwrap_or_else(|| Self::site_rate(site, direction))
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

    fn site_override_update_from_state(site: &SiteState) -> SiteOverrideUpdate {
        SiteOverrideUpdate {
            node_id: site.config.node_id.clone(),
            site_name: site.config.name.clone(),
            download_bandwidth_mbps: (site.queue_download_mbps != site.config.max_download_mbps)
                .then_some(site.queue_download_mbps as f32),
            upload_bandwidth_mbps: (site.queue_upload_mbps != site.config.max_upload_mbps)
                .then_some(site.queue_upload_mbps as f32),
        }
    }

    fn site_override_update_with_target(
        site: &SiteState,
        direction: RecommendationDirection,
        target_mbps: u64,
    ) -> SiteOverrideUpdate {
        let mut update = Self::site_override_update_from_state(site);
        match direction {
            RecommendationDirection::Download => {
                update.download_bandwidth_mbps =
                    (target_mbps != site.config.max_download_mbps).then_some(target_mbps as f32);
            }
            RecommendationDirection::Upload => {
                update.upload_bandwidth_mbps =
                    (target_mbps != site.config.max_upload_mbps).then_some(target_mbps as f32);
            }
        }
        update
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

    fn record_failed_application(
        site: &mut SiteState,
        direction: RecommendationDirection,
        action: RecommendationAction,
        target_mbps: u64,
        error: String,
    ) {
        site.record_attempt(
            direction,
            ActionAttempt {
                action,
                target_mbps,
                outcome: "failed".to_string(),
                unix_ms: unix_time_ms(),
                error: Some(error),
            },
        );
    }

    fn record_skipped_application(
        site: &mut SiteState,
        recommendation: &Recommendation,
        reason: &str,
    ) {
        let target_mbps = Self::attempt_target(site, recommendation.direction);
        site.record_attempt(
            recommendation.direction,
            ActionAttempt {
                action: recommendation.action,
                target_mbps,
                outcome: "skipped".to_string(),
                unix_ms: unix_time_ms(),
                error: Some(reason.to_string()),
            },
        );
    }

    fn complete_successful_application(
        site: &mut SiteState,
        direction: RecommendationDirection,
        action: RecommendationAction,
        target_mbps: u64,
        cooldown_secs: f32,
        dry_run: bool,
    ) {
        if !dry_run {
            Self::set_site_rate(site, direction, target_mbps);
        }
        site.record_attempt(
            direction,
            ActionAttempt {
                action,
                target_mbps,
                outcome: successful_application_outcome(dry_run).to_string(),
                unix_ms: unix_time_ms(),
                error: None,
            },
        );
        debug!("Recommendation applied: entering cooldown");
        Self::enter_cooldown(site, direction, cooldown_secs, action);
    }
}

/// K1b §4.6 pin 3: mints the generation the next topology publish will produce
/// for the just-written clamp layer, then waits for the bridge to settle it. A
/// clamp is only acknowledged once the bridge APPLIED it; a REJECTED or
/// timed-out publish fails loud and the caller restores the pre-write snapshot.
pub(crate) async fn publish_clamp_and_wait(
    bus_tx: &StormguardBusSender,
    publish_wait_seconds: u64,
) -> anyhow::Result<()> {
    let global = lqos_config::load_config()
        .context("could not load config for the StormGuard clamp mint")?;
    let generation = lqos_topology::mint_next_shaping_generation(&global)?
        .ok_or_else(|| anyhow!("topology produced no shaping generation for the StormGuard clamp"))?;
    match crate::bus::wait_for_publish(
        bus_tx.clone(),
        &generation,
        Duration::from_secs(publish_wait_seconds.max(1)),
    )
    .await?
    {
        BridgePublishStatus::Applied => Ok(()),
        BridgePublishStatus::Rejected { reason } => Err(anyhow!(
            "bridge rejected the StormGuard clamp generation: {reason}"
        )),
        other => Err(anyhow!(
            "unexpected settled StormGuard clamp publish state: {other:?}"
        )),
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
                node_id: None,
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
            passive_rtt_flow_counts: (0, 0),
            rtt_sample_for_baseline_ms: None,
            rtt_baseline_ms: None,
            last_passive_rtt_ms: None,
            last_passive_rtt_at: None,
            passive_rtt_updated_this_tick: false,
            last_action_download: None,
            last_action_upload: None,
            download_decision: DirectionDecision::default(),
            upload_decision: DirectionDecision::default(),
            last_attempt_download: None,
            last_attempt_upload: None,
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
            override_ttl_seconds: 300,
            publish_wait_seconds: 30,
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
        assert_eq!(update.download_bandwidth_mbps, Some(75.0));
        assert_eq!(update.upload_bandwidth_mbps, None);
    }

    #[test]
    fn successful_outcomes_distinguish_dry_run_from_live_application() {
        assert_eq!(successful_application_outcome(true), "dry_run");
        assert_eq!(successful_application_outcome(false), "applied");
    }

    #[test]
    fn active_rtt_source_respects_endpoint_weights() {
        assert_eq!(active_rtt_source(Some(20.0), Some(40.0), 0.0), "passive");
        assert_eq!(active_rtt_source(Some(20.0), Some(40.0), 0.5), "blended");
        assert_eq!(active_rtt_source(Some(20.0), Some(40.0), 1.0), "active");
    }

    #[test]
    fn failed_application_keeps_rate_and_remains_out_of_cooldown() {
        let mut site = site_state(50, 25, 100, 50);
        SiteStateTracker::record_failed_application(
            &mut site,
            RecommendationDirection::Download,
            RecommendationAction::Decrease,
            45,
            "synthetic Bakery failure".to_string(),
        );

        assert_eq!(site.queue_download_mbps, 50);
        assert_eq!(site.download_state, StormguardState::Running);
        let attempt = site
            .last_attempt(RecommendationDirection::Download)
            .expect("failed attempt");
        assert_eq!(attempt.outcome, "failed");
        assert_eq!(attempt.target_mbps, 45);
        assert_eq!(attempt.error.as_deref(), Some("synthetic Bakery failure"));
    }

    #[test]
    fn acknowledged_live_application_updates_rate_and_enters_cooldown() {
        let mut site = site_state(50, 25, 100, 50);
        SiteStateTracker::complete_successful_application(
            &mut site,
            RecommendationDirection::Download,
            RecommendationAction::Decrease,
            45,
            3.0,
            false,
        );

        assert_eq!(site.queue_download_mbps, 45);
        assert!(matches!(
            site.download_state,
            StormguardState::Cooldown { .. }
        ));
        assert_eq!(
            site.last_attempt(RecommendationDirection::Download)
                .map(|attempt| attempt.outcome.as_str()),
            Some("applied")
        );
    }

    #[test]
    fn dry_run_records_outcome_without_changing_rate() {
        let mut site = site_state(50, 25, 100, 50);
        SiteStateTracker::complete_successful_application(
            &mut site,
            RecommendationDirection::Download,
            RecommendationAction::Decrease,
            45,
            3.0,
            true,
        );

        assert_eq!(site.queue_download_mbps, 50);
        assert!(matches!(
            site.download_state,
            StormguardState::Cooldown { .. }
        ));
        assert_eq!(
            site.last_attempt(RecommendationDirection::Download)
                .map(|attempt| attempt.outcome.as_str()),
            Some("dry_run")
        );
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
        let decision = site.decision(RecommendationDirection::Download);
        assert!(decision.score.is_none());
        assert!(decision.target_mbps.is_some());
        assert!(decision.reason.contains("decrease 40.0"));
        assert!(decision.reason.starts_with("severe delay or retransmit signal"));
        assert!(!decision.reason.contains("Some("));
        assert!(!decision.reason.contains("None"));
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
        let reason = &site
            .decision(RecommendationDirection::Download)
            .reason;
        assert!(reason.contains("probe requirements"));
        assert!(reason.contains("current probe age=11 ticks"));
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

    #[test]
    fn cooldown_records_candidate_without_emitting_recommendation() {
        let cfg = test_config(StormguardStrategy::DelayProbe);
        let mut site = site_state(20, 20, 50, 50);
        site.download_state = StormguardState::Cooldown {
            start: Instant::now(),
            duration_secs: 60.0,
        };
        site.current_throughput = (18.0, 0.0);
        site.current_rtt_ms = Some(900.0);
        site.rtt_baseline_ms = Some(600.0);

        let mut recs = Vec::new();
        site.recommendations(&mut recs, &cfg);
        assert!(
            !recs
                .iter()
                .any(|(r, _)| { r.direction == RecommendationDirection::Download })
        );
        let decision = site.decision(RecommendationDirection::Download);
        assert_eq!(decision.blocker.as_deref(), Some("cooldown"));
        assert!(decision.candidate_action.is_some());
        assert!(decision.target_mbps.is_some());
    }

    #[test]
    fn legacy_strategy_exposes_its_numeric_score() {
        let cfg = test_config(StormguardStrategy::LegacyScore);
        let mut site = site_state(20, 20, 50, 50);
        site.current_throughput = (1.0, 1.0);
        let mut recs = Vec::new();
        site.recommendations(&mut recs, &cfg);
        let download = site.decision(RecommendationDirection::Download);
        assert!(download.score.is_some());
        assert!(download.reason.contains("legacy score inputs"));
        assert!(!download.reason.contains("Some("));
        assert!(!download.reason.contains("None"));
        assert!(
            site.decision(RecommendationDirection::Upload)
                .score
                .is_some()
        );
    }

}
