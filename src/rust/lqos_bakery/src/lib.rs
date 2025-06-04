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

mod tc_control;
mod utils;
mod commands;

use std::path::Path;
use std::sync::{Arc, Mutex};
use crossbeam_channel::Receiver;
use tracing::{debug, error, info, warn};
use utils::current_timestamp;
pub (crate) const CHANNEL_CAPACITY: usize = 65536; // 64k capacity for Bakery commands
pub use commands::BakeryCommands;


/// Starts the Bakery system, returning a channel sender for sending commands to the Bakery.
pub fn start_bakery() -> anyhow::Result<crossbeam_channel::Sender<BakeryCommands>> {
    let (tx, rx) = crossbeam_channel::bounded(CHANNEL_CAPACITY);
    std::thread::Builder::new()
        .name("lqos_bakery".to_string())
        .spawn(move || {
            bakery_main(rx);
        })
        .map_err(|e| anyhow::anyhow!("Failed to start Bakery thread: {}", e))?;
    Ok(tx)
}


fn bakery_main(rx: Receiver<BakeryCommands>) {
    let Ok(config) = lqos_config::load_config() else {
        error!("Failed to load configuration, exiting Bakery thread.");
        return;
    };
    let lazy_queues_enabled = config.queues.lazy_queues.unwrap_or(false);
    let queue_expiration_time_seconds = config.queues.lazy_expire_seconds.unwrap_or(600); // Default to 10 minutes

    while let Ok(command) = rx.recv() {
        debug!("Bakery received command: {:?}", command);
    }
    error!("Bakery thread exited unexpectedly.");
}
