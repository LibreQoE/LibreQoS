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

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use crossbeam_channel::{Receiver, Sender};
use tracing::{debug, error, info, warn};
use utils::current_timestamp;
pub (crate) const CHANNEL_CAPACITY: usize = 65536; // 64k capacity for Bakery commands
pub use commands::BakeryCommands;
use lqos_config::Config;

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

                let new_batch = batch.take(); // Take the batch to avoid cloning
                if let Some(new_batch) = new_batch {
                    process_batch(new_batch, &config);
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
        }
    }
    error!("Bakery thread exited unexpectedly.");
}

fn process_batch(batch: Vec<BakeryCommands>, config: &Arc<lqos_config::Config>) {
    info!("Bakery: Processing batch of {} commands", batch.len());
    let commands = batch
        .into_iter()
        .map(|b| b.to_commands(config))
        .flatten()
        .flatten()
        .collect::<Vec<Vec<String>>>();

    if write_command_file(&config, commands) {
        // Something bad happened while writing the command file
        return;
    }

    let path = Path::new(&config.lqos_directory)
        .join("linux_tc_rust.txt");
    let path_str = path.to_string_lossy().to_string();

    info!("Bakery: Command file written successfully at {}", path_str);
    // /sbin/tc -f -b linux_tc.txt
    std::process::Command::new("/sbin/tc")
        .arg("-f") // Force the command to run
        .arg("-b") // Batch mode
        .arg(path_str) // Path to the command file
        .spawn()
        .map_err(|e| error!("Failed to execute tc command: {}", e))
        .ok(); // Ignore errors, as we just want to run the command
}

fn write_command_file(config: &&Arc<Config>, commands: Vec<Vec<String>>) -> bool {
    // Output file
    let path = Path::new(&config.lqos_directory)
        .join("linux_tc_rust.txt");
    let Ok(f) = File::create(&path) else {
        error!("Failed to create output file: {}", path.display());
        return true;
    };
    let mut f = BufWriter::new(f);
    for line in commands {
        for (idx, entry) in line.iter().enumerate() {
            if let Err(e) = f.write_all(entry.as_bytes()) {
                error!("Failed to write to output file: {}", e);
                return true;
            }
            if idx < line.len() - 1 {
                if let Err(e) = f.write_all(b" ") {
                    error!("Failed to write space to output file: {}", e);
                    return true;
                }
            }
        }
        let newline = "\n";
        if let Err(e) = f.write_all(newline.as_bytes()) {
            error!("Failed to write newline to output file: {}", e);
            return true;
        }
    }
    false
}