mod analysis;
mod recommendation;
mod ring_buffer;
mod site;
mod stormguard_state;

use crate::active_ping::TimedRtt;
use crate::adaptive_actions::{
    CircuitFallbackOutcome, SiteOverrideUpdate, apply_circuit_fallback,
    apply_site_override_updates, clear_circuit_fallback, clear_stormguard_override_layer,
    load_persisted_circuit_fallbacks, wait_for_bakery_reply,
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
use crossbeam_channel::Sender;
use lqos_bakery::{BakeryCommands, StormGuardClassAdjustment, StormGuardRestoreAdjustment};
use lqos_bus::{StormguardDebugDirection, StormguardDebugEntry, TcHandle};
use lqos_config::NetworkJsonTransport;
use lqos_overrides::{NetworkAdjustment, OverrideFile, OverrideLayer, OverrideStore};
use lqos_queue_tracker::QUEUE_STRUCTURE;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

pub struct SiteStateTracker {
    tree_generation: u64,
    sites: HashMap<String, SiteState>,
    active_circuit_fallbacks: HashSet<String>,
}

struct CircuitQueueRecommendationContext<'a> {
    tree_generation: u64,
    active_circuit_fallbacks: &'a mut HashSet<String>,
    site: &'a mut SiteState,
    config: &'a StormguardConfig,
    recommendation: &'a Recommendation,
    circuit_id: &'a str,
    cooldown_secs: f32,
    bakery_sender: Sender<BakeryCommands>,
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

async fn reset_bakery_adjustments(
    tree_generation: u64,
    adjustments: Vec<StormGuardRestoreAdjustment>,
    restore_untracked: bool,
    bakery_sender: Sender<BakeryCommands>,
) -> anyhow::Result<()> {
    let (reply_sender, reply_receiver) = std::sync::mpsc::channel();
    bakery_sender
        .send(BakeryCommands::ResetStormGuardAdjustments {
            tree_generation,
            adjustments,
            restore_untracked,
            reply: Some(reply_sender),
        })
        .context("failed to send StormGuard reset command to Bakery")?;

    wait_for_bakery_reply(reply_receiver, "reset StormGuard adjustments").await
}

/// Discards class ownership cached against the previous shaping tree.
///
/// Side effects: clears Bakery's in-process StormGuard adjustment cache after acknowledgement.
pub(crate) async fn discard_bakery_adjustments(
    tree_generation: u64,
    bakery_sender: Sender<BakeryCommands>,
) -> anyhow::Result<()> {
    let (reply_sender, reply_receiver) = std::sync::mpsc::channel();
    bakery_sender
        .send(BakeryCommands::DiscardStormGuardAdjustments {
            tree_generation,
            reply: Some(reply_sender),
        })
        .context("failed to send StormGuard discard command to Bakery")?;
    wait_for_bakery_reply(reply_receiver, "discard stale StormGuard ownership").await
}

/// Clears circuit fallbacks retained across an incremental tracker reload.
///
/// Side effects: applies acknowledged Bakery circuit updates and removes any matching persisted
/// StormGuard device overrides for each retained circuit.
pub(crate) async fn clear_retained_circuit_fallbacks(
    circuit_ids: &HashSet<String>,
    tree_generation: u64,
    bakery_sender: Sender<BakeryCommands>,
) -> anyhow::Result<()> {
    for circuit_id in circuit_ids {
        match clear_circuit_fallback(circuit_id, false, tree_generation, bakery_sender.clone())
            .await?
        {
            CircuitFallbackOutcome::Cleared { .. } | CircuitFallbackOutcome::Skipped { .. } => {}
            outcome => {
                return Err(anyhow!(
                    "unexpected retained circuit cleanup outcome for {circuit_id}: {outcome:?}"
                ));
            }
        }
    }
    Ok(())
}

/// Clears stale StormGuard replay and persistence state while StormGuard is inactive.
///
/// Side effects: asks Bakery to discard retained HTB replay entries, then empties the persisted
/// StormGuard override layer. The persisted layer is unchanged if Bakery cannot acknowledge.
pub async fn reconcile_inactive_state(bakery_sender: Sender<BakeryCommands>) -> anyhow::Result<()> {
    let tree_generation = lqos_bakery::stormguard_tree_generation();
    let override_snapshot = OverrideStore::load_layer(OverrideLayer::Stormguard)?;
    let adjustments = persisted_restore_adjustments(&override_snapshot)?;
    if lqos_bakery::stormguard_tree_generation() != tree_generation {
        return Err(anyhow!(
            "Bakery shaping tree changed while preparing inactive StormGuard reconciliation"
        ));
    }
    reset_bakery_adjustments(tree_generation, adjustments, true, bakery_sender.clone()).await?;
    let circuit_ids: Vec<String> = load_persisted_circuit_fallbacks()?.into_keys().collect();
    for circuit_id in circuit_ids {
        let outcome =
            clear_circuit_fallback(&circuit_id, false, tree_generation, bakery_sender.clone())
                .await;
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                restore_override_snapshot(
                    &override_snapshot,
                    "after inactive circuit cleanup failed",
                )?;
                return Err(error);
            }
        };
        match outcome {
            CircuitFallbackOutcome::Cleared { .. } | CircuitFallbackOutcome::Skipped { .. } => {}
            outcome => {
                restore_override_snapshot(
                    &override_snapshot,
                    "after unexpected inactive circuit cleanup outcome",
                )?;
                return Err(anyhow!(
                    "unexpected inactive reset outcome for circuit {circuit_id}: {outcome:?}"
                ));
            }
        }
    }
    ensure_generation_or_restore_overrides(
        tree_generation,
        &override_snapshot,
        "before inactive StormGuard persistence cleanup",
    )?;
    clear_stormguard_override_layer().context("failed to clear inactive StormGuard overrides")?;
    ensure_generation_or_restore_overrides(
        tree_generation,
        &override_snapshot,
        "during inactive StormGuard persistence cleanup",
    )?;
    Ok(())
}

fn ensure_generation_or_restore_overrides(
    expected_generation: u64,
    snapshot: &OverrideFile,
    context: &str,
) -> anyhow::Result<()> {
    if lqos_bakery::stormguard_tree_generation() == expected_generation {
        return Ok(());
    }
    restore_override_snapshot(snapshot, &format!("after tree changed {context}"))?;
    Err(anyhow!("Bakery shaping tree changed {context}"))
}

fn restore_override_snapshot(snapshot: &OverrideFile, context: &str) -> anyhow::Result<()> {
    OverrideStore::save_layer(OverrideLayer::Stormguard, snapshot)
        .with_context(|| format!("failed to restore StormGuard ownership {context}"))
}

fn persisted_restore_adjustments(
    overrides: &OverrideFile,
) -> anyhow::Result<Vec<StormGuardRestoreAdjustment>> {
    let site_adjustments: Vec<_> = overrides
        .network_adjustments()
        .iter()
        .filter_map(|adjustment| match adjustment {
            NetworkAdjustment::AdjustSiteSpeed {
                site_name,
                download_bandwidth_mbps,
                upload_bandwidth_mbps,
                ..
            } => Some((
                site_name.clone(),
                *download_bandwidth_mbps,
                *upload_bandwidth_mbps,
            )),
            _ => None,
        })
        .collect();
    if site_adjustments.is_empty() {
        return Ok(Vec::new());
    }

    let config = lqos_config::load_config()?;
    let queue_snapshot = QUEUE_STRUCTURE.load();
    let queues = queue_snapshot
        .maybe_queues
        .as_ref()
        .ok_or_else(|| anyhow!("queue structure is unavailable for inactive StormGuard reset"))?;
    let mut adjustments: HashMap<(String, TcHandle), (u64, u64)> = HashMap::new();

    for (site_name, download_target, upload_target) in site_adjustments {
        let Some(queue) = queues
            .iter()
            .find(|queue| queue.name.as_deref() == Some(site_name.as_str()))
        else {
            warn!(
                "Skipping stale persisted StormGuard site {site_name}: its queue no longer exists"
            );
            continue;
        };
        let dependencies = crate::queue_structure::find_queue_dependents(&site_name)?;
        for (direction, target) in [
            (RecommendationDirection::Download, download_target),
            (RecommendationDirection::Upload, upload_target),
        ] {
            let Some(target) = target else {
                continue;
            };
            let interface = match direction {
                RecommendationDirection::Download => config.isp_interface(),
                RecommendationDirection::Upload => config.internet_interface(),
            };
            let planned_rate = match direction {
                RecommendationDirection::Download => queue.download_bandwidth_mbps_min,
                RecommendationDirection::Upload => queue.upload_bandwidth_mbps_min,
            };
            let planned_ceil = match direction {
                RecommendationDirection::Download => queue.download_bandwidth_mbps,
                RecommendationDirection::Upload => queue.upload_bandwidth_mbps,
            };
            adjustments.insert(
                (interface.clone(), queue.class_id),
                (planned_rate, planned_ceil),
            );
            for dependency in &dependencies {
                let (dependency_rate, dependency_ceil) = match direction {
                    RecommendationDirection::Download => (
                        dependency.original_min_download_mbps,
                        dependency.original_max_download_mbps,
                    ),
                    RecommendationDirection::Upload => (
                        dependency.original_min_upload_mbps,
                        dependency.original_max_upload_mbps,
                    ),
                };
                if dependency_ceil >= target.max(0.0) as u64 {
                    adjustments.insert(
                        (interface.clone(), dependency.class_id),
                        (dependency_rate, dependency_ceil),
                    );
                }
            }
        }
    }

    Ok(adjustments
        .into_iter()
        .map(
            |((interface_name, class_id), (planned_rate, planned_ceil))| {
                StormGuardRestoreAdjustment {
                    interface_name,
                    class_id,
                    planned_rate,
                    planned_ceil,
                }
            },
        )
        .collect())
}

impl SiteStateTracker {
    pub(crate) fn active_circuit_fallbacks(&self) -> HashSet<String> {
        self.active_circuit_fallbacks.clone()
    }

    pub(crate) fn retain_active_circuit_fallbacks(&mut self, circuit_ids: HashSet<String>) {
        self.active_circuit_fallbacks.extend(circuit_ids);
    }

    pub fn from_config(config: &StormguardConfig, tree_generation: u64) -> Self {
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
            tree_generation,
            sites,
            active_circuit_fallbacks: HashSet::new(),
        }
    }

    pub async fn replay_persisted_adjustments(
        &mut self,
        config: &StormguardConfig,
        bakery_sender: Sender<BakeryCommands>,
    ) -> anyhow::Result<()> {
        if config.dry_run {
            return Ok(());
        }

        let Some(queues) = &QUEUE_STRUCTURE.load().maybe_queues else {
            info!("No queue structure - cannot replay StormGuard adjustments");
            return Ok(());
        };

        let fallbacks = load_persisted_circuit_fallbacks()?;
        for (circuit_id, fallback) in fallbacks {
            match apply_circuit_fallback(
                &circuit_id,
                &fallback.sqm_override,
                false,
                false,
                self.tree_generation,
                bakery_sender.clone(),
            )
            .await?
            {
                CircuitFallbackOutcome::Applied { .. } => {
                    self.active_circuit_fallbacks.insert(circuit_id.clone());
                }
                CircuitFallbackOutcome::Skipped { reason } => warn!(
                    "Skipped persisted StormGuard circuit fallback for {circuit_id}: {reason}"
                ),
                outcome => {
                    return Err(anyhow!(
                        "unexpected persisted circuit replay outcome for {circuit_id}: {outcome:?}"
                    ));
                }
            }
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
                let adjustments = Self::site_adjustment_batch(
                    &site.config,
                    direction,
                    planned_rate,
                    current_rate,
                    &interface_name,
                    queue.class_id,
                    Self::queue_minimum_rate(queue, direction),
                );
                Self::send_adjustment_batch_confirmed(
                    self.tree_generation,
                    config.dry_run,
                    adjustments,
                    bakery_sender.clone(),
                )
                .await?;
            }
        }
        Ok(())
    }

    /// Restores StormGuard-managed classes and circuit fallbacks to their planned state.
    ///
    /// Side effects: applies live Bakery commands and empties the persisted StormGuard override
    /// layer. Persisted and replay state is cleared only after the live HTB reset succeeds.
    pub async fn reset_adjustments(
        &mut self,
        bakery_sender: Sender<BakeryCommands>,
    ) -> anyhow::Result<()> {
        let override_snapshot = OverrideStore::load_layer(OverrideLayer::Stormguard)?;
        reset_bakery_adjustments(
            self.tree_generation,
            Vec::new(),
            false,
            bakery_sender.clone(),
        )
        .await?;

        let active_circuit_fallbacks: Vec<String> =
            self.active_circuit_fallbacks.iter().cloned().collect();
        for circuit_id in active_circuit_fallbacks {
            match clear_circuit_fallback(
                &circuit_id,
                false,
                self.tree_generation,
                bakery_sender.clone(),
            )
            .await
            {
                Ok(CircuitFallbackOutcome::Cleared { .. }) => {}
                Ok(CircuitFallbackOutcome::Skipped { reason }) => {
                    warn!(
                        "StormGuard circuit fallback for {circuit_id} no longer owns the live SQM setting: {reason}"
                    );
                }
                Ok(outcome) => {
                    restore_override_snapshot(
                        &override_snapshot,
                        "after unexpected circuit cleanup outcome",
                    )?;
                    return Err(anyhow!(
                        "unexpected reset outcome for StormGuard circuit fallback {circuit_id}: {outcome:?}"
                    ));
                }
                Err(error) => {
                    restore_override_snapshot(&override_snapshot, "after circuit cleanup failed")?;
                    return Err(error).with_context(|| {
                        format!("failed to clear StormGuard circuit fallback {circuit_id}")
                    });
                }
            }
        }

        ensure_generation_or_restore_overrides(
            self.tree_generation,
            &override_snapshot,
            "before StormGuard persistence cleanup",
        )?;
        clear_stormguard_override_layer()
            .context("failed to clear persisted StormGuard adjustments")?;
        ensure_generation_or_restore_overrides(
            self.tree_generation,
            &override_snapshot,
            "during StormGuard persistence cleanup",
        )?;
        self.active_circuit_fallbacks.clear();
        Ok(())
    }

    pub fn read_new_tick_data(
        &mut self,
        config: &StormguardConfig,
        active_ping_sample: Option<TimedRtt>,
        active_ping_updated: bool,
        all_nodes: Vec<(usize, NetworkJsonTransport)>,
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
        bakery_sender: Sender<BakeryCommands>,
    ) -> ApplicationReport {
        let mut application_errors = Vec::new();
        let mut acknowledged_success = false;
        // We'll need the queues to apply HTB commands
        let Some(queues) = &QUEUE_STRUCTURE.load().maybe_queues else {
            info!("No queue structure - cannot get stats");
            application_errors.push(
                "StormGuard cannot apply recommendations because queue state is unavailable"
                    .to_string(),
            );
            for (recommendation, _) in recommendations {
                if let Some(site) = self.sites.get_mut(&recommendation.site) {
                    Self::record_skipped_application(
                        site,
                        &recommendation,
                        "queue structure unavailable",
                    );
                }
            }
            return ApplicationReport {
                errors: application_errors,
                acknowledged_success,
            };
        };

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

            // Find the Queue Object
            let Some(queue) = queues.iter().find(|n| {
                if let Some(q) = &n.name {
                    *q == recommendation.site
                } else {
                    false
                }
            }) else {
                info!("Queue {} not found in queue structure", recommendation.site);
                Self::record_skipped_application(
                    site,
                    &recommendation,
                    "watched queue is absent from the shaping tree",
                );
                application_errors.push(format!(
                    "StormGuard watched queue {} is absent from the shaping tree",
                    recommendation.site
                ));
                continue;
            };
            let cooldown_secs = Self::cooldown_for_action(config, &recommendation.action);
            debug!(
                "Cooldown for {:?} set to {:.1}s",
                recommendation.action, cooldown_secs
            );

            // Circuit queues host qdiscs; prefer the TreeGuard-style fallback path.
            if let Some(circuit_id) = queue.circuit_id.as_deref() {
                if let Some(error) =
                    Self::handle_circuit_queue_recommendation(CircuitQueueRecommendationContext {
                        tree_generation: self.tree_generation,
                        active_circuit_fallbacks: &mut self.active_circuit_fallbacks,
                        site,
                        config,
                        recommendation: &recommendation,
                        circuit_id,
                        cooldown_secs,
                        bakery_sender: bakery_sender.clone(),
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

            let interface_name = Self::interface_name(config, recommendation.direction);

            // Find the TC class
            let class_handle = queue.class_id;

            // Find the new bandwidth
            let current_rate = Self::site_rate(site, recommendation.direction);
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

            let adjustments = Self::site_adjustment_batch(
                &site.config,
                recommendation.direction,
                current_rate,
                new_rate,
                &interface_name,
                class_handle,
                Self::queue_minimum_rate(queue, recommendation.direction),
            );
            let apply_result = Self::send_adjustment_batch_confirmed(
                self.tree_generation,
                config.dry_run,
                adjustments.clone(),
                bakery_sender.clone(),
            )
            .await;
            if let Err(error) = apply_result {
                Self::record_failed_application(
                    site,
                    recommendation.direction,
                    recommendation.action,
                    new_rate,
                    error.to_string(),
                );
                warn!(
                    "StormGuard {} adjustment for {} failed: {}",
                    recommendation.direction, recommendation.site, error
                );
                application_errors.push(error.to_string());
                continue;
            }

            if !config.dry_run {
                let update = Self::site_override_update_with_target(
                    site,
                    recommendation.direction,
                    new_rate,
                );
                if let Err(error) = apply_site_override_updates(&[update]) {
                    let rollback_adjustments = adjustments
                        .into_iter()
                        .map(|adjustment| StormGuardClassAdjustment {
                            new_rate: adjustment.previous_rate,
                            previous_rate: adjustment.new_rate,
                            ..adjustment
                        })
                        .collect();
                    let rollback_error = Self::send_adjustment_batch_confirmed(
                        self.tree_generation,
                        false,
                        rollback_adjustments,
                        bakery_sender.clone(),
                    )
                    .await
                    .err();
                    let error = match rollback_error {
                        Some(rollback_error) => format!(
                            "failed to persist StormGuard rate: {error}; rollback failed: {rollback_error}"
                        ),
                        None => format!(
                            "failed to persist StormGuard rate: {error}; live classes restored"
                        ),
                    };
                    Self::record_failed_application(
                        site,
                        recommendation.direction,
                        recommendation.action,
                        new_rate,
                        error.clone(),
                    );
                    warn!(
                        "StormGuard persistence failed for {}: {error}",
                        recommendation.site
                    );
                    application_errors.push(error);
                    continue;
                }
            }
            Self::complete_successful_application(
                site,
                recommendation.direction,
                recommendation.action,
                new_rate,
                cooldown_secs,
                config.dry_run,
            );
            acknowledged_success = true;
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
            tree_generation,
            active_circuit_fallbacks,
            site,
            config,
            recommendation,
            circuit_id,
            cooldown_secs,
            bakery_sender,
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
            match recommendation.action {
                RecommendationAction::Decrease | RecommendationAction::DecreaseFast => {
                    apply_circuit_fallback(
                        circuit_id,
                        &config.circuit_fallback_sqm,
                        config.circuit_fallback_persist,
                        config.dry_run,
                        tree_generation,
                        bakery_sender,
                    )
                    .await
                    .map_err(|error| format!("Fallback error: {error}"))
                }
                RecommendationAction::Increase | RecommendationAction::IncreaseFast => {
                    clear_circuit_fallback(
                        circuit_id,
                        config.dry_run,
                        tree_generation,
                        bakery_sender,
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
            Ok(CircuitFallbackOutcome::AppliedUnpersisted { error }) => {
                active_circuit_fallbacks.insert(circuit_id.to_string());
                warn!(
                    "StormGuard circuit fallback is live but unhealthy for {} ({}): {}",
                    recommendation.site, circuit_id, error
                );
                (
                    format!("circuit_fallback=applied_unpersisted error={error}"),
                    false,
                    "failed",
                    Some(error),
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

    fn queue_minimum_rate(
        queue: &lqos_queue_tracker::QueueNode,
        direction: RecommendationDirection,
    ) -> u64 {
        match direction {
            RecommendationDirection::Download => queue.download_bandwidth_mbps_min,
            RecommendationDirection::Upload => queue.upload_bandwidth_mbps_min,
        }
    }

    fn interface_name(config: &StormguardConfig, direction: RecommendationDirection) -> String {
        match direction {
            RecommendationDirection::Download => config.download_interface.clone(),
            RecommendationDirection::Upload => config.upload_interface.clone(),
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

    fn site_adjustment_batch(
        site: &crate::config::WatchingSite,
        direction: RecommendationDirection,
        previous_rate: u64,
        new_rate: u64,
        interface_name: &str,
        site_class_id: TcHandle,
        site_planned_rate: u64,
    ) -> Vec<StormGuardClassAdjustment> {
        let mut adjustments: Vec<StormGuardClassAdjustment> =
            Self::dependent_adjustments(site, direction, new_rate)
                .into_iter()
                .map(
                    |(class_id, planned_rate, planned_ceil, rate)| StormGuardClassAdjustment {
                        interface_name: interface_name.to_string(),
                        class_id,
                        new_rate: rate,
                        previous_rate: previous_rate.min(planned_ceil),
                        planned_rate,
                        planned_ceil,
                    },
                )
                .collect();
        adjustments.push(StormGuardClassAdjustment {
            interface_name: interface_name.to_string(),
            class_id: site_class_id,
            new_rate,
            previous_rate,
            planned_rate: site_planned_rate,
            planned_ceil: Self::planned_rate(site, direction),
        });
        adjustments
    }

    fn dependent_adjustments(
        site: &crate::config::WatchingSite,
        direction: RecommendationDirection,
        target_rate: u64,
    ) -> Vec<(TcHandle, u64, u64, u64)> {
        site.dependent_nodes
            .iter()
            .map(|dependent| {
                let original_rate = match direction {
                    RecommendationDirection::Download => dependent.original_max_download_mbps,
                    RecommendationDirection::Upload => dependent.original_max_upload_mbps,
                };
                let original_min_rate = match direction {
                    RecommendationDirection::Download => dependent.original_min_download_mbps,
                    RecommendationDirection::Upload => dependent.original_min_upload_mbps,
                };
                (
                    dependent.class_id,
                    original_min_rate,
                    original_rate,
                    target_rate.min(original_rate),
                )
            })
            .collect()
    }

    fn site_override_update_from_state(site: &SiteState) -> SiteOverrideUpdate {
        SiteOverrideUpdate {
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

    async fn send_adjustment_batch_confirmed(
        tree_generation: u64,
        dry_run: bool,
        adjustments: Vec<StormGuardClassAdjustment>,
        bakery_sender: Sender<BakeryCommands>,
    ) -> anyhow::Result<()> {
        let (reply_sender, reply_receiver) = std::sync::mpsc::channel();
        bakery_sender
            .send(BakeryCommands::StormGuardAdjustmentBatch {
                tree_generation,
                dry_run,
                adjustments,
                reply: reply_sender,
            })
            .context("failed to send StormGuard adjustment batch to Bakery")?;
        wait_for_bakery_reply(reply_receiver, "apply StormGuard adjustment batch").await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StormguardConfig as RuntimeStormguardConfig;
    use crate::config::{WatchingSite, WatchingSiteDependency};
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
        assert_eq!(update.download_bandwidth_mbps, Some(75.0));
        assert_eq!(update.upload_bandwidth_mbps, None);
    }

    #[test]
    fn dependent_adjustment_carries_original_directional_rate_pair() -> anyhow::Result<()> {
        let mut site = site_state(50, 50, 100, 100).config;
        site.dependent_nodes.push(WatchingSiteDependency {
            name: "Dependent A".to_string(),
            class_id: TcHandle::from_string("1:2")?,
            original_min_download_mbps: 7,
            original_min_upload_mbps: 3,
            original_max_download_mbps: 37,
            original_max_upload_mbps: 19,
        });
        let download =
            SiteStateTracker::dependent_adjustments(&site, RecommendationDirection::Download, 15);
        let upload =
            SiteStateTracker::dependent_adjustments(&site, RecommendationDirection::Upload, 15);
        assert_eq!(download, [(TcHandle::from_string("1:2")?, 7, 37, 15)]);
        assert_eq!(upload, [(TcHandle::from_string("1:2")?, 3, 19, 15)]);
        Ok(())
    }

    #[test]
    fn dependent_adjustment_restores_classes_only_to_their_planned_ceiling() -> anyhow::Result<()> {
        let mut site = site_state(50, 50, 100, 100).config;
        site.dependent_nodes.push(WatchingSiteDependency {
            name: "Dependent A".to_string(),
            class_id: TcHandle::from_string("1:2")?,
            original_min_download_mbps: 7,
            original_min_upload_mbps: 3,
            original_max_download_mbps: 37,
            original_max_upload_mbps: 19,
        });
        assert_eq!(
            SiteStateTracker::dependent_adjustments(&site, RecommendationDirection::Download, 40,),
            [(TcHandle::from_string("1:2")?, 7, 37, 37)]
        );
        Ok(())
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

    #[tokio::test]
    async fn acknowledged_batch_reply_propagates_success_and_failure() -> anyhow::Result<()> {
        let adjustment = StormGuardClassAdjustment {
            interface_name: "eth0".to_string(),
            class_id: TcHandle::from_string("1:2")?,
            new_rate: 50,
            previous_rate: 60,
            planned_rate: 10,
            planned_ceil: 100,
        };

        for expected in [Ok(()), Err("synthetic failure".to_string())] {
            let (sender, receiver) = crossbeam_channel::bounded(1);
            let expected_reply = expected.clone();
            let worker = std::thread::spawn(move || {
                let BakeryCommands::StormGuardAdjustmentBatch { reply, .. } =
                    receiver.recv().expect("batch command")
                else {
                    panic!("unexpected Bakery command");
                };
                reply.send(expected_reply).expect("batch reply");
            });
            let result = SiteStateTracker::send_adjustment_batch_confirmed(
                1,
                false,
                vec![adjustment.clone()],
                sender,
            )
            .await;
            worker.join().expect("Bakery reply worker");
            match expected {
                Ok(()) => assert!(result.is_ok()),
                Err(message) => assert!(
                    result
                        .expect_err("failure should propagate")
                        .to_string()
                        .contains(&message)
                ),
            }
        }
        Ok(())
    }
}
