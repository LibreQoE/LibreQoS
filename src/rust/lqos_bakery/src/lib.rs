//! The Bakery is where CAKE is made!
//!
//! More specifically, this crate provides a tracker of TC queues - described by the LibreQoS.py process,
//! but tracked for changes. We're at phase 3.
//!
//! In phase 1, the Bakery will build queues and a matching structure to track them. It will act exactly
//! like the LibreQoS.py process.
//!
//! In phase 2, the Bakery will *not* create CAKE queues - just the HTB hierarchy. When circuits are
//! detected as having traffic, the associated queue will be created. Ideally, some form of timeout
//! will be used to remove queues that are no longer in use. (Saving resources)
//!
//! In phase 3, the Bakery will - after initial creation - track the queues and update them as needed.
//! This will take a "diff" approach, finding differences and only applying those changes.
//!
//! In phase 4, the Bakery will implement "live move" --- allowing queues to be moved losslessly. This will
//! complete the NLNet project goals.

mod commands;
mod diff;
mod queue_math;
mod utils;

use crossbeam_channel::{Receiver, Sender};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use tracing::{debug, error, info, warn};
use utils::current_timestamp;
pub(crate) const CHANNEL_CAPACITY: usize = 65536; // 64k capacity for Bakery commands
use crate::commands::ExecutionMode;
use crate::diff::{SiteDiffResult, diff_sites};
use crate::queue_math::format_rate_for_tc_f32;
use crate::utils::{execute_in_memory, write_command_file};
pub use commands::BakeryCommands;
use lqos_config::{Config, LazyQueueMode};

pub static ACTIVE_CIRCUITS: AtomicUsize = AtomicUsize::new(0);

pub static BAKERY_SENDER: OnceLock<Sender<BakeryCommands>> = OnceLock::new();
static MQ_CREATED: AtomicBool = AtomicBool::new(false);

/// Starts the Bakery system, returning a channel sender for sending commands to the Bakery.
pub fn start_bakery() -> anyhow::Result<crossbeam_channel::Sender<BakeryCommands>> {
    let (tx, rx) = crossbeam_channel::bounded(CHANNEL_CAPACITY);
    let inner_sender = tx.clone();
    if BAKERY_SENDER.set(tx.clone()).is_err() {
        return Err(anyhow::anyhow!("Bakery sender is already initialized."));
    }
    std::thread::Builder::new()
        .name("lqos_bakery".to_string())
        .spawn(move || {
            bakery_main(rx, inner_sender);
        })
        .map_err(|e| anyhow::anyhow!("Failed to start Bakery thread: {}", e))?;
    Ok(tx)
}

fn bakery_main(rx: Receiver<BakeryCommands>, tx: Sender<BakeryCommands>) {
    // Current operation batch
    let mut batch: Option<Vec<Arc<BakeryCommands>>> = None;
    let mut sites: HashMap<i64, Arc<BakeryCommands>> = HashMap::new();
    let mut circuits: HashMap<i64, Arc<BakeryCommands>> = HashMap::new();
    let mut live_circuits: HashMap<i64, u64> = HashMap::new();

    {
        let Ok(config) = lqos_config::load_config() else {
            error!("Failed to load configuration, exiting Bakery thread.");
            return;
        };
        info!(
            "Bakery thread starting. Mode: {:?}, expiration: {}s",
            config.queues.lazy_queues,
            config.queues.lazy_expire_seconds.unwrap_or(600)
        );
    }

    while let Ok(command) = rx.recv() {
        debug!("Bakery received command: {:?}", command);

        match command {
            BakeryCommands::StartBatch => {
                batch = Some(Vec::new());
            }
            BakeryCommands::CommitBatch => {
                handle_commit_batch(
                    &mut batch,
                    &mut sites,
                    &mut circuits,
                    &mut live_circuits,
                    &tx,
                );
            }
            BakeryCommands::MqSetup { .. } => {
                if let Some(batch) = &mut batch {
                    batch.push(Arc::new(command));
                }
            }
            BakeryCommands::AddSite { .. } => {
                if let Some(batch) = &mut batch {
                    batch.push(Arc::new(command));
                }
            }
            BakeryCommands::AddCircuit { .. } => {
                if let Some(batch) = &mut batch {
                    batch.push(Arc::new(command));
                }
            }
            BakeryCommands::OnCircuitActivity { circuit_ids } => {
                handle_circuit_activity(circuit_ids, &circuits, &mut live_circuits);
            }
            BakeryCommands::Tick => {
                // Reset per-cycle counters at the start of tick
                handle_tick(&mut circuits, &mut live_circuits, &mut sites);
            }
            BakeryCommands::ChangeSiteSpeedLive {
                site_hash,
                download_bandwidth_min,
                upload_bandwidth_min,
                download_bandwidth_max,
                upload_bandwidth_max,
            } => {
                handle_change_site_speed_live(
                    site_hash,
                    download_bandwidth_min,
                    upload_bandwidth_min,
                    download_bandwidth_max,
                    upload_bandwidth_max,
                    &mut sites,
                );
            }
            BakeryCommands::StormGuardAdjustment {
                dry_run,
                interface_name,
                class_id,
                new_rate,
            } => {
                let has_mq_run = MQ_CREATED.load(Relaxed);
                if !has_mq_run {
                    warn!("StormGuardAdjustment received before MQ setup, skipping.");
                    continue;
                }
                // Build the HTB command
                let args = vec![
                    "class".to_string(),
                    "change".to_string(),
                    "dev".to_string(),
                    interface_name.to_string(),
                    "classid".to_string(),
                    class_id.to_string(),
                    "htb".to_string(),
                    "rate".to_string(),
                    format!("{}mbit", new_rate - 1),
                    "ceil".to_string(),
                    format!("{}mbit", new_rate),
                ];
                if dry_run {
                    warn!("DRY RUN: /sbin/tc {}", args.join(" "));
                } else {
                    let output = std::process::Command::new("/sbin/tc").args(&args).output();
                    match output {
                        Err(e) => {
                            warn!("Failed to run tc command: {}", e);
                        }
                        Ok(out) => {
                            if !out.status.success() {
                                warn!(
                                    "tc command failed: {}",
                                    String::from_utf8_lossy(&out.stderr)
                                );
                            } else {
                                info!(
                                    "tc command succeeded: {}",
                                    String::from_utf8_lossy(&out.stdout)
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    error!("Bakery thread exited unexpectedly.");
}

fn handle_commit_batch(
    batch: &mut Option<Vec<Arc<BakeryCommands>>>,
    sites: &mut HashMap<i64, Arc<BakeryCommands>>,
    circuits: &mut HashMap<i64, Arc<BakeryCommands>>,
    live_circuits: &mut HashMap<i64, u64>,
    tx: &Sender<BakeryCommands>,
) {
    let Ok(config) = lqos_config::load_config() else {
        error!("Failed to load configuration, exiting Bakery thread.");
        return;
    };

    let Some(new_batch) = batch.take() else {
        warn!("CommitBatch received without a batch to commit.");
        return;
    };

    let has_mq_been_setup = MQ_CREATED.load(std::sync::atomic::Ordering::Relaxed);
    if !has_mq_been_setup {
        // If the MQ hasn't been created, we need to do this as a full, unadjusted run.
        info!("MQ not created, performing full reload.");
        full_reload(batch, sites, circuits, live_circuits, &config, new_batch);
        MQ_CREATED.store(true, std::sync::atomic::Ordering::Relaxed);
        return;
    }

    let site_change_mode = diff_sites(&new_batch, &sites);
    if matches!(site_change_mode, SiteDiffResult::RebuildRequired) {
        // If the site structure has changed, we need to rebuild everything.
        info!("Site structure has changed, performing full reload.");
        full_reload(batch, sites, circuits, live_circuits, &config, new_batch);
        MQ_CREATED.store(true, std::sync::atomic::Ordering::Relaxed);
        return;
    }

    let circuit_change_mode = diff::diff_circuits(&new_batch, &circuits);

    // If neither has changed, there's nothing to do.
    if matches!(site_change_mode, SiteDiffResult::NoChange)
        && matches!(circuit_change_mode, diff::CircuitDiffResult::NoChange)
    {
        // No changes detected, skip processing
        info!("No changes detected in batch, skipping processing.");
        return;
    }

    // Check if we should do a full reload based on the number of circuit changes
    if let diff::CircuitDiffResult::CircuitsChanged {
        newly_added: _,
        removed_circuits: _,
        updated_circuits: _,
    } = &circuit_change_mode
    {
        full_reload(batch, sites, circuits, live_circuits, &config, new_batch);
        return; // Skip the rest of this CommitBatch processing
    }

    // Declare any site speed changes that need to be applied. We're sending them
    // to ourselves as future commands via the BakeryCommands channel.
    if let SiteDiffResult::SpeedChanges { changes } = site_change_mode {
        for change in &changes {
            let BakeryCommands::AddSite {
                site_hash,
                download_bandwidth_min,
                upload_bandwidth_min,
                download_bandwidth_max,
                upload_bandwidth_max,
                ..
            } = change
            else {
                warn!(
                    "ChangeSiteSpeedLive received a non-site command: {:?}",
                    change
                );
                continue;
            };
            if let Err(e) = tx.try_send(BakeryCommands::ChangeSiteSpeedLive {
                site_hash: *site_hash,
                download_bandwidth_min: *download_bandwidth_min,
                upload_bandwidth_min: *upload_bandwidth_min,
                download_bandwidth_max: *download_bandwidth_max,
                upload_bandwidth_max: *upload_bandwidth_max,
            }) {
                error!("Channel full, falling back to full rebuild: {}", e);
                full_reload(
                    batch,
                    sites,
                    circuits,
                    live_circuits,
                    &config,
                    new_batch.clone(),
                );
                return; // Skip the rest of this CommitBatch processing
            }
        }
    }

    // Now we can process circuit changes
    if let diff::CircuitDiffResult::CircuitsChanged {
        newly_added,
        removed_circuits,
        updated_circuits,
    } = circuit_change_mode
    {
        // Process removed circuits (including those that are being updated)
        let mut circuits_to_remove = removed_circuits;
        if !updated_circuits.is_empty() {
            // For updates, we need to remove the old version first
            for cmd in &updated_circuits {
                if let BakeryCommands::AddCircuit { circuit_hash, .. } = cmd.as_ref() {
                    circuits_to_remove.push(*circuit_hash);
                }
            }
        }

        // Removing circuits.
        if !circuits_to_remove.is_empty() {
            for circuit_hash in circuits_to_remove {
                if let Some(circuit) = circuits.remove(&circuit_hash) {
                    let was_activated = live_circuits.contains_key(&circuit_hash);

                    // Only generate removal commands if appropriate for the mode
                    let commands = match config.queues.lazy_queues.as_ref() {
                        None | Some(LazyQueueMode::No) => {
                            // Non-lazy: everything was created, delete everything
                            circuit.to_prune(&config, true)
                        }
                        Some(LazyQueueMode::Htb) => {
                            // HTB mode: only delete CAKE if it was created
                            if was_activated {
                                circuit.to_prune(&config, false) // This will only delete CAKE, not HTB
                            } else {
                                None // CAKE was never created, nothing to delete
                            }
                        }
                        Some(LazyQueueMode::Full) => {
                            // Full lazy: only delete if activated
                            if was_activated {
                                circuit.to_prune(&config, true)
                            } else {
                                None // Nothing was created, nothing to delete
                            }
                        }
                    };

                    if let Some(cmd) = commands {
                        execute_in_memory(&cmd, "removing circuit");
                    }
                    live_circuits.remove(&circuit_hash);
                } else {
                    warn!(
                        "RemoveCircuit received for unknown circuit: {}",
                        circuit_hash
                    );
                    continue;
                }
            }
        }
        // Newly added is a little harder
        if !newly_added.is_empty() {
            // Collect both newly added and updated circuits
            let mut all_new_circuits: Vec<&Arc<BakeryCommands>> = newly_added;
            all_new_circuits.extend(updated_circuits);

            let commands: Vec<Vec<String>> = all_new_circuits
                .iter()
                .map(|c| c.to_commands(&config, ExecutionMode::Builder))
                .flatten()
                .flatten()
                .collect();
            if commands.is_empty() {
                debug!("No commands to execute for newly added circuits.");
            } else {
                execute_in_memory(&commands, "adding new circuits");
                // Update the circuits map with the newly added circuits
                for command in all_new_circuits {
                    if let BakeryCommands::AddCircuit { circuit_hash, .. } = command.as_ref() {
                        circuits.insert(*circuit_hash, Arc::clone(command));
                    } else {
                        warn!("AddCircuit received a non-circuit command: {:?}", command);
                    }
                }
            }
        }
    }
}

fn handle_circuit_activity(
    circuit_ids: HashSet<i64>,
    circuits: &HashMap<i64, Arc<BakeryCommands>>,
    live_circuits: &mut HashMap<i64, u64>,
) {
    let Ok(config) = lqos_config::load_config() else {
        error!("Failed to load configuration, exiting Bakery thread.");
        return;
    };
    match config.queues.lazy_queues.as_ref() {
        None | Some(LazyQueueMode::No) => return,
        _ => {}
    }

    let mut commands = Vec::new();
    for circuit_id in circuit_ids {
        if let Some(circuit) = live_circuits.get_mut(&circuit_id) {
            *circuit = current_timestamp();
            continue;
        }

        if let Some(command) = circuits.get(&circuit_id) {
            let Some(cmd) = command.to_commands(&config, ExecutionMode::LiveUpdate) else {
                continue;
            };
            live_circuits.insert(circuit_id, current_timestamp());
            commands.extend(cmd);
        }
    }
    if commands.is_empty() {
        return; // No commands to write
    }
    execute_in_memory(&commands, "enabling live circuits");
}

fn handle_tick(
    circuits: &mut HashMap<i64, Arc<BakeryCommands>>,
    live_circuits: &mut HashMap<i64, u64>,
    sites: &mut HashMap<i64, Arc<BakeryCommands>>,
) {
    // This is a periodic tick to expire lazy queues
    let Ok(config) = lqos_config::load_config() else {
        error!("Failed to load configuration, exiting Bakery thread.");
        return;
    };

    // Periodically shrink HashMap capacity if it's much larger than needed
    static mut TICK_COUNT: u64 = 0;
    unsafe {
        TICK_COUNT += 1;
        if TICK_COUNT % 60 == 0 {
            // Every minute
            // Shrink if capacity is more than 2x the size
            if circuits.capacity() > circuits.len() * 2 && circuits.capacity() > 100 {
                debug!(
                    "Shrinking circuits HashMap: {} entries, {} capacity",
                    circuits.len(),
                    circuits.capacity()
                );
                circuits.shrink_to_fit();
            }
            if live_circuits.capacity() > live_circuits.len() * 2 && live_circuits.capacity() > 100
            {
                debug!(
                    "Shrinking live_circuits HashMap: {} entries, {} capacity",
                    live_circuits.len(),
                    live_circuits.capacity()
                );
                live_circuits.shrink_to_fit();
            }
            if sites.capacity() > sites.len() * 2 && sites.capacity() > 100 {
                debug!(
                    "Shrinking sites HashMap: {} entries, {} capacity",
                    sites.len(),
                    sites.capacity()
                );
                sites.shrink_to_fit();
            }
        }
    }

    match config.queues.lazy_queues.as_ref() {
        None | Some(LazyQueueMode::No) => {
            ACTIVE_CIRCUITS.store(circuits.len(), Ordering::Relaxed);
            return;
        }
        _ => {
            ACTIVE_CIRCUITS.store(live_circuits.len(), Ordering::Relaxed);
        }
    }

    // Now we know that lazy queues are enabled, we can expire them!
    let max_age_seconds = config.queues.lazy_expire_seconds.unwrap_or(600);
    if max_age_seconds == 0 {
        // If max_age_seconds is 0, we do not expire queues
        return;
    }

    let mut to_destroy = Vec::new();
    let now = current_timestamp();
    for (circuit_id, last_activity) in live_circuits.iter() {
        if now - *last_activity > max_age_seconds {
            to_destroy.push(*circuit_id);
        }
    }

    if to_destroy.is_empty() {
        return; // No queues to expire
    }

    let mut commands = Vec::new();
    for circuit_id in to_destroy {
        if let Some(command) = circuits.get(&circuit_id) {
            let Some(cmd) = command.to_prune(&config, false) else {
                continue;
            };
            live_circuits.remove(&circuit_id);
            commands.extend(cmd);
        }
    }

    if commands.is_empty() {
        return; // No commands to write
    }
    execute_in_memory(&commands, "pruning lazy queues");
}

fn handle_change_site_speed_live(
    site_hash: i64,
    download_bandwidth_min: f32,
    upload_bandwidth_min: f32,
    download_bandwidth_max: f32,
    upload_bandwidth_max: f32,
    sites: &mut HashMap<i64, Arc<BakeryCommands>>,
) {
    let Ok(config) = lqos_config::load_config() else {
        error!("Failed to load configuration, exiting Bakery thread.");
        return;
    };
    if let Some(site_arc) = sites.get(&site_hash) {
        let BakeryCommands::AddSite {
            site_hash: _,
            parent_class_id,
            up_parent_class_id,
            class_minor,
            ..
        } = site_arc.as_ref()
        else {
            warn!(
                "ChangeSiteSpeedLive received a non-site command: {:?}",
                site_arc
            );
            return;
        };
        let to_internet = config.internet_interface();
        let to_isp = config.isp_interface();
        let class_id = format!(
            "0x{:x}:0x{:x}",
            parent_class_id.get_major_minor().0,
            class_minor
        );
        let up_class_id = format!(
            "0x{:x}:0x{:x}",
            up_parent_class_id.get_major_minor().0,
            class_minor
        );
        let upload_bandwidth_min = if upload_bandwidth_min >= (upload_bandwidth_max - 0.5) {
            upload_bandwidth_max - 1.0
        } else {
            upload_bandwidth_min
        };
        let download_bandwidth_min = if download_bandwidth_min >= (download_bandwidth_max - 0.5) {
            download_bandwidth_max - 1.0
        } else {
            download_bandwidth_min
        };
        let commands = vec![
            vec![
                "class".to_string(),
                "change".to_string(),
                "dev".to_string(),
                to_internet,
                "classid".to_string(),
                up_class_id,
                "htb".to_string(),
                "rate".to_string(),
                format_rate_for_tc_f32(upload_bandwidth_min),
                "ceil".to_string(),
                format_rate_for_tc_f32(upload_bandwidth_max),
            ],
            vec![
                "class".to_string(),
                "change".to_string(),
                "dev".to_string(),
                to_isp,
                "classid".to_string(),
                class_id,
                "htb".to_string(),
                "rate".to_string(),
                format_rate_for_tc_f32(download_bandwidth_min),
                "ceil".to_string(),
                format_rate_for_tc_f32(download_bandwidth_max),
            ],
        ];
        execute_in_memory(&commands, "changing site speed live");
        // Update the site speeds in the site map - create a new Arc with updated values
        let new_site = Arc::new(BakeryCommands::AddSite {
            site_hash,
            parent_class_id: *parent_class_id,
            up_parent_class_id: *up_parent_class_id,
            class_minor: *class_minor,
            download_bandwidth_min,
            upload_bandwidth_min,
            download_bandwidth_max,
            upload_bandwidth_max,
        });
        sites.insert(site_hash, new_site);
    } else {
        warn!(
            "ChangeSiteSpeedLive received for unknown site: {}",
            site_hash
        );
        return;
    }
}

fn full_reload(
    batch: &mut Option<Vec<Arc<BakeryCommands>>>,
    sites: &mut HashMap<i64, Arc<BakeryCommands>>,
    circuits: &mut HashMap<i64, Arc<BakeryCommands>>,
    live_circuits: &mut HashMap<i64, u64>,
    config: &Arc<Config>,
    new_batch: Vec<Arc<BakeryCommands>>,
) {
    warn!("Bakery: Full reload triggered due to site or circuit changes.");
    sites.clear();
    circuits.clear();
    live_circuits.clear();
    process_batch(new_batch, &config, sites, circuits);
    *batch = None;
}

fn process_batch(
    batch: Vec<Arc<BakeryCommands>>,
    config: &Arc<lqos_config::Config>,
    sites: &mut HashMap<i64, Arc<BakeryCommands>>,
    circuits: &mut HashMap<i64, Arc<BakeryCommands>>,
) {
    info!("Bakery: Processing batch of {} commands", batch.len());
    let mut circuit_count = 0u64;
    let commands = batch
        .into_iter()
        .map(|b| {
            // Ensure that our state map is up to date with the latest commands
            match b.as_ref() {
                BakeryCommands::AddSite { site_hash, .. } => {
                    sites.insert(*site_hash, Arc::clone(&b));
                }
                BakeryCommands::AddCircuit { circuit_hash, .. } => {
                    circuits.insert(*circuit_hash, Arc::clone(&b));
                    circuit_count += 1;
                }
                _ => {}
            }
            b.to_commands(config, ExecutionMode::Builder)
        })
        .flatten()
        .flatten()
        .collect::<Vec<Vec<String>>>();

    let path = Path::new(&config.lqos_directory).join("linux_tc_rust.txt");
    write_command_file(&path, &commands);
    execute_in_memory(&commands, "processing batch");
}
