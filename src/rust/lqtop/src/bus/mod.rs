//! Handles the communication loop with lqosd.

use crate::ui_base::SHOULD_EXIT;
use anyhow::{bail, Result};
use lqos_bus::{BusClient, BusRequest, BusResponse};
use std::sync::atomic::Ordering;
use tokio::sync::mpsc::{Receiver, Sender};
pub mod cpu_ram;
pub mod throughput;

/// Event types to instruct the bus
pub enum BusCommand {
    /// Collect the total throughput
    CollectTotalThroughput(bool),
    /// Quit the bus
    Quit,
}

/// The main loop for the bus.
/// Spawns a separate task to handle the bus communication.
pub async fn bus_loop() -> Sender<BusCommand> {
    let (tx, rx) = tokio::sync::mpsc::channel::<BusCommand>(100);

    tokio::spawn(cpu_ram::gather_sysinfo());
    tokio::spawn(main_loop_wrapper(rx));

    tx
}

async fn main_loop_wrapper(rx: Receiver<BusCommand>) {
    let loop_result = main_loop(rx).await;
    if let Err(e) = loop_result {
        eprintln!("Error in main loop: {}", e);
        SHOULD_EXIT.store(true, Ordering::Relaxed);
    }
}

async fn main_loop(mut rx: Receiver<BusCommand>) -> Result<()> {
    // Collection Settings
    let mut collect_total_throughput = true;

    let mut bus_client = BusClient::new().await?;
    if !bus_client.is_connected() {
        bail!("Failed to connect to the bus");
    }

    loop {
        // Do we have any behavior changing commands?
        if let Ok(cmd) = rx.try_recv() {
            match cmd {
                BusCommand::CollectTotalThroughput(val) => {
                    collect_total_throughput = val;
                }
                BusCommand::Quit => {
                    SHOULD_EXIT.store(true, Ordering::Relaxed);
                    break;
                }
            }
        }

        // Perform actual bus collection
        let mut commands: Vec<BusRequest> = Vec::new();

        if collect_total_throughput {
            commands.push(BusRequest::GetCurrentThroughput);
        }

        // Send the requests and process replies
        for response in bus_client.request(commands).await? {
            match response {
                BusResponse::CurrentThroughput { .. } => throughput::throughput(&response).await,
                _ => {}
            }
        }

        // Check if we should be quitting
        if SHOULD_EXIT.load(Ordering::Relaxed) {
            break;
        }

        // Sleep for one tick
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    Ok(())
}
