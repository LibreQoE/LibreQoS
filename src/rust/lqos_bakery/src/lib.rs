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

mod utils;
mod commands;
mod queue_math;
mod diff;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};
use std::sync::atomic::AtomicBool;
use crossbeam_channel::{Receiver, Sender};
use tracing::{debug, error, info, warn};
use utils::current_timestamp;
pub (crate) const CHANNEL_CAPACITY: usize = 65536; // 64k capacity for Bakery commands
pub use commands::BakeryCommands;
use lqos_config::{Config, LazyQueueMode};
use crate::commands::ExecutionMode;
use crate::diff::{diff_sites, SiteDiffResult};
use crate::queue_math::format_rate_for_tc_f32;
use crate::utils::execute_in_memory;

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
    let mut batch = None;
    let mut sites = HashMap::new();
    let mut circuits = HashMap::new();
    let mut live_circuits = HashMap::new();

    while let Ok(command) = rx.recv() {
        debug!("Bakery received command: {:?}", command);

        match command {
            BakeryCommands::StartBatch => {
                batch = Some(Vec::new());
            },
            BakeryCommands::CommitBatch => {
                handle_commit_batch(
                    &mut batch,
                    &mut sites,
                    &mut circuits,
                    &mut live_circuits,
                    &tx,
                );
            },
            BakeryCommands::MqSetup { .. } => {
                if let Some(batch) = &mut batch {
                    batch.push(command.clone());
                }
            },
            BakeryCommands::AddSite { .. } => {
                if let Some(batch) = &mut batch {
                    batch.push(command.clone());
                }
            }
            BakeryCommands::AddCircuit { .. } => {
                if let Some(batch) = &mut batch {
                    batch.push(command.clone());
                }
            }
            BakeryCommands::OnCircuitActivity { circuit_ids } => {
                handle_circuit_activity(circuit_ids, &circuits, &mut live_circuits);
            }
            BakeryCommands::Tick => {
                handle_tick(&circuits, &mut live_circuits);
            }
            BakeryCommands::ChangeSiteSpeedLive { 
                site_hash, 
                download_bandwidth_min, 
                upload_bandwidth_min, 
                download_bandwidth_max, 
                upload_bandwidth_max 
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
        }
    }
    error!("Bakery thread exited unexpectedly.");
}

fn handle_commit_batch(
    batch: &mut Option<Vec<BakeryCommands>>,
    sites: &mut HashMap<i64, BakeryCommands>,
    circuits: &mut HashMap<i64, BakeryCommands>,
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
        full_reload(batch, sites, circuits, live_circuits, &config, new_batch);
        return;
    }

    let site_change_mode = diff_sites(&new_batch, &sites);
    if matches!(site_change_mode, SiteDiffResult::RebuildRequired) {
        // If the site structure has changed, we need to rebuild everything.
        full_reload(batch, sites, circuits, live_circuits, &config, new_batch);
        return;
    }

    let circuit_change_mode = diff::diff_circuits(&new_batch, &circuits);

    // If neither has changed, there's nothing to do.
    if matches!(site_change_mode, SiteDiffResult::NoChange) && matches!(circuit_change_mode, diff::CircuitDiffResult::NoChange) {
        // No changes detected, skip processing
        debug!("No changes detected in batch, skipping processing.");
        return;
    }

    // Declare any site speed changes that need to be applied. We're sending them
    // to ourselves as future commands via the BakeryCommands channel.
    if let SiteDiffResult::SpeedChanges { changes } = site_change_mode {
        if changes.is_empty() {
            debug!("No speed changes detected, skipping processing.");
            return;
        }

        for change in &changes {
            let BakeryCommands::AddSite { site_hash, download_bandwidth_min, upload_bandwidth_min, download_bandwidth_max, upload_bandwidth_max, .. } = change else {
                warn!("ChangeSiteSpeedLive received a non-site command: {:?}", change);
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
                full_reload(batch, sites, circuits, live_circuits, &config, new_batch.clone());
                return; // Skip the rest of this CommitBatch processing
            }
        }
    }

    // Now we can process circuit changes
    if let diff::CircuitDiffResult::CircuitsChanged { mut newly_added, mut removed_circuits, updated_circuits } = circuit_change_mode {
        // And updated is annoying. Because it's really the other two steps,
        // let's split the data and pass it to the other two commands.
        if !updated_circuits.is_empty() {
            removed_circuits.extend(updated_circuits.clone());
            newly_added.extend(updated_circuits);
        }

        // Removing circuits.
        if !removed_circuits.is_empty() {
            for to_remove in removed_circuits {
                let BakeryCommands::AddCircuit { circuit_hash, .. } = &to_remove else {
                    warn!("RemoveCircuit received a non-circuit command: {:?}", to_remove);
                    continue;
                };
                if let Some(circuit) = circuits.remove(circuit_hash) {
                    let commands = circuit.to_prune(&config, true);
                    if let Some(cmd) = commands {
                        execute_in_memory(&cmd, "removing circuit");
                    }
                    live_circuits.remove(circuit_hash);
                } else {
                    warn!("RemoveCircuit received for unknown circuit: {}", circuit_hash);
                    continue;
                }
            }
        }
        // Newly added is a little harder
        if !newly_added.is_empty() {
            let commands: Vec<Vec<String>> = newly_added
                .iter()
                .map(|c| c.to_commands(&config, ExecutionMode::Builder))
                .flatten()
                .flatten()
                .collect();
            if commands.is_empty() {
                debug!("No commands to execute for newly added circuits.");
            } else {
                execute_in_memory(&commands, "adding new circuits");
            }
        }
    }
}

fn handle_circuit_activity(
    circuit_ids: HashSet<i64>,
    circuits: &HashMap<i64, BakeryCommands>,
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
    circuits: &HashMap<i64, BakeryCommands>,
    live_circuits: &mut HashMap<i64, u64>,
) {
    // This is a periodic tick to expire lazy queues
    let Ok(config) = lqos_config::load_config() else {
        error!("Failed to load configuration, exiting Bakery thread.");
        return;
    };
    match config.queues.lazy_queues.as_ref() {
        None | Some(LazyQueueMode::No) => return,
        _ => {}
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
    sites: &mut HashMap<i64, BakeryCommands>,
) {
    let Ok(config) = lqos_config::load_config() else {
        error!("Failed to load configuration, exiting Bakery thread.");
        return;
    };
    if let Some(site) = sites.get_mut(&site_hash) {
        let BakeryCommands::AddSite { site_hash: _, parent_class_id, up_parent_class_id, class_minor, download_bandwidth_min: site_dl_min, upload_bandwidth_min: site_ul_min, download_bandwidth_max: site_dl_max, upload_bandwidth_max: site_ul_max } = site else {
            warn!("ChangeSiteSpeedLive received a non-site command: {:?}", site);
            return;
        };
        let to_internet = config.internet_interface();
        let to_isp = config.isp_interface();
        let class_id = format!("0x{:x}:0x{:x}", parent_class_id.get_major_minor().0, class_minor);
        let up_class_id = format!("0x{:x}:0x{:x}", up_parent_class_id.get_major_minor().0, class_minor);
        let commands = vec![vec![
            "tc".to_string(),
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
        ], vec![
            "tc".to_string(),
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
        ]];
        execute_in_memory(&commands, "changing site speed live");
        // Update the site speeds in the site map
        *site_dl_min = download_bandwidth_min;
        *site_ul_min = upload_bandwidth_min;
        *site_dl_max = download_bandwidth_max;
        *site_ul_max = upload_bandwidth_max;
    } else {
        warn!("ChangeSiteSpeedLive received for unknown site: {}", site_hash);
        return;
    }
}

fn full_reload(batch: &mut Option<Vec<BakeryCommands>>, sites: &mut HashMap<i64, BakeryCommands>, circuits: &mut HashMap<i64, BakeryCommands>, live_circuits: &mut HashMap<i64, u64>, config: &Arc<Config>, new_batch: Vec<BakeryCommands>) {
    sites.clear();
    circuits.clear();
    live_circuits.clear();
    process_batch(new_batch, &config, sites, circuits);
    *batch = None;
    MQ_CREATED.store(true, std::sync::atomic::Ordering::Relaxed);
}

fn process_batch(
    batch: Vec<BakeryCommands>,
    config: &Arc<lqos_config::Config>,
    sites: &mut HashMap<i64, BakeryCommands>,
    circuits: &mut HashMap<i64, BakeryCommands>,
) {
    info!("Bakery: Processing batch of {} commands", batch.len());
    let commands = batch
        .into_iter()
        .map(|b| {
            // Ensure that our state map is up to date with the latest commands
            match &b {
                BakeryCommands::AddSite { site_hash, .. } => {
                    sites.insert(*site_hash, b.clone());
                }
                BakeryCommands::AddCircuit { circuit_hash, .. } => {
                    circuits.insert(*circuit_hash, b.clone());
                }
                _ => {}
            }
            b.to_commands(config, ExecutionMode::Builder)
        })
        .flatten()
        .flatten()
        .collect::<Vec<Vec<String>>>();

    execute_in_memory(&commands, "processing batch");
}

