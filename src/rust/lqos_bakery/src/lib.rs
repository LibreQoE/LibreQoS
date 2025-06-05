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

use std::collections::HashMap;
use std::io::Write;
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use crossbeam_channel::{Receiver, Sender};
use tracing::{debug, error, info};
use utils::current_timestamp;
pub (crate) const CHANNEL_CAPACITY: usize = 65536; // 64k capacity for Bakery commands
pub use commands::BakeryCommands;
use lqos_config::LazyQueueMode;
use crate::commands::ExecutionMode;

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

fn execute_in_memory(command_buffer: &Vec<Vec<String>>, purpose: &str) {
    info!("Bakery: Executing in-memory commands: {} lines, for {purpose}", command_buffer.len());

    for line in command_buffer {
        let Ok(output) = std::process::Command::new("/sbin/tc")
            .args(line)
            .output() else {
                error!("Failed to execute command: {:?}", line);
                continue;
            };
        //println!("/sbin/tc/{}", line.join(" "));
        let output_str = String::from_utf8_lossy(&output.stdout);
        if !output_str.is_empty() {
            error!("Executing command: {:?}", line);
            error!("Command result: {:?}", output_str.trim());
        }
        let error_str = String::from_utf8_lossy(&output.stderr);
        if !error_str.is_empty() {
            error!("Executing command: {:?}", line);
            error!("Command error: {:?}", error_str.trim());
        }
    }

    // Commented out because it didn't appear to be faster, and you lose the ability to see individual command errors
    /*let mut commands = String::new();
    for line in command_buffer {
        for (idx, entry) in line.iter().enumerate() {
            commands.push_str(entry);
            if idx < line.len() - 1 {
                commands.push(' '); // Add space between entries
            }
        }
        let newline = "\n";
        commands.push_str(newline); // Add new-line at the end of the line
    }

    let Ok(mut child) = std::process::Command::new("/sbin/tc")
        .arg("-batch")  // or "-force" if you want it to continue after errors
        .arg("-")       // read from stdin
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .inspect_err(|e| {
            error!("Failed to spawn tc command: {}", e);
        }) else {
            return;
        };

    {
        let stdin = child.stdin.as_mut().expect("Failed to open stdin");
        if let Err(e) = stdin.write_all(commands.as_bytes()) {
            error!("Failed to write to tc stdin: {}", e);
            return;
        }
    }

    let Ok(status) = child.wait() else {
        error!("Failed to wait for tc command to finish");
        return;
    };
    if !status.success() {
        eprintln!("tc command failed with status: {}", status);
    }*/
}

// fn write_command_file(path: &Path, commands: Vec<Vec<String>>) -> bool {
//     let Ok(f) = File::create(path) else {
//         error!("Failed to create output file: {}", path.display());
//         return true;
//     };
//     let mut f = BufWriter::new(f);
//     for line in commands {
//         for (idx, entry) in line.iter().enumerate() {
//             if let Err(e) = f.write_all(entry.as_bytes()) {
//                 error!("Failed to write to output file: {}", e);
//                 return true;
//             }
//             if idx < line.len() - 1 {
//                 if let Err(e) = f.write_all(b" ") {
//                     error!("Failed to write space to output file: {}", e);
//                     return true;
//                 }
//             }
//         }
//         let newline = "\n";
//         if let Err(e) = f.write_all(newline.as_bytes()) {
//             error!("Failed to write newline to output file: {}", e);
//             return true;
//         }
//     }
//     false
// }