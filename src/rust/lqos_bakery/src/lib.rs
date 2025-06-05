//! The Bakery is where CAKE is made!
//! 
//! More specifically, this crate provides a tracker of TC queues - described by the LibreQoS.py process,
//! but tracked for changes. We're at phase 2.
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

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use crossbeam_channel::{Receiver, Sender};
use tracing::{debug, error, info};
use utils::current_timestamp;
pub (crate) const CHANNEL_CAPACITY: usize = 65536; // 64k capacity for Bakery commands
pub use commands::BakeryCommands;
use lqos_config::LazyQueueMode;
use crate::commands::ExecutionMode;
use crate::utils::execute_in_memory;

pub static BAKERY_SENDER: OnceLock<Sender<BakeryCommands>> = OnceLock::new();

/// Starts the Bakery system, returning a channel sender for sending commands to the Bakery.
pub fn start_bakery() -> anyhow::Result<crossbeam_channel::Sender<BakeryCommands>> {
    let (tx, rx) = crossbeam_channel::bounded(CHANNEL_CAPACITY);
    if BAKERY_SENDER.set(tx.clone()).is_err() {
        return Err(anyhow::anyhow!("Bakery sender is already initialized."));
    }
    std::thread::Builder::new()
        .name("lqos_bakery".to_string())
        .spawn(move || {
            bakery_main(rx);
        })
        .map_err(|e| anyhow::anyhow!("Failed to start Bakery thread: {}", e))?;
    Ok(tx)
}


fn bakery_main(rx: Receiver<BakeryCommands>) {
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
                let Ok(config) = lqos_config::load_config() else {
                    error!("Failed to load configuration, exiting Bakery thread.");
                    continue;
                };

                sites.clear();
                circuits.clear();
                live_circuits.clear();
                let new_batch = batch.take(); // Take the batch to avoid cloning
                if let Some(new_batch) = new_batch {
                    process_batch(new_batch, &config, &mut sites, &mut circuits);
                }
                batch = None; // Clear the batch after committing
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
                let Ok(config) = lqos_config::load_config() else {
                    error!("Failed to load configuration, exiting Bakery thread.");
                    continue;
                };
                match config.queues.lazy_queues.as_ref() {
                    None | Some(LazyQueueMode::No) => continue,
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
                    continue; // No commands to write
                }
                execute_in_memory(&commands, "enabling live circuits");
            }
            BakeryCommands::Tick => {
                // This is a periodic tick to expire lazy queues
                let Ok(config) = lqos_config::load_config() else {
                    error!("Failed to load configuration, exiting Bakery thread.");
                    continue;
                };
                match config.queues.lazy_queues.as_ref() {
                    None | Some(LazyQueueMode::No) => continue,
                    _ => {}
                }

                // Now we know that lazy queues are enabled, we can expire them!
                let max_age_seconds = config.queues.lazy_expire_seconds.unwrap_or(600);
                if max_age_seconds == 0 {
                    // If max_age_seconds is 0, we do not expire queues
                    continue;
                }

                let mut to_destroy = Vec::new();
                let now = current_timestamp();
                for (circuit_id, last_activity) in live_circuits.iter() {
                    if now - *last_activity > max_age_seconds {
                        to_destroy.push(*circuit_id);
                    }
                }

                if to_destroy.is_empty() {
                    continue; // No queues to expire
                }

                let mut commands = Vec::new();
                for circuit_id in to_destroy {
                    if let Some(command) = circuits.get(&circuit_id) {
                        let Some(cmd) = command.to_prune(&config) else {
                            continue;
                        };
                        live_circuits.remove(&circuit_id);
                        commands.extend(cmd);
                    }
                }

                if commands.is_empty() {
                    continue; // No commands to write
                }
                execute_in_memory(&commands, "pruning lazy queues");
            }
        }
    }
    error!("Bakery thread exited unexpectedly.");
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

