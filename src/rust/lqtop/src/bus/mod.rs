//! Handles the communication loop with lqosd.

use crate::ui_base::SHOULD_EXIT;
use anyhow::{bail, Result};
use lqos_bus::{BusClient, BusRequest, BusResponse};
use std::sync::atomic::Ordering;
pub mod cpu_ram;
pub mod throughput;
pub mod top_hosts;

/// The main loop for the bus.
/// Spawns a separate task to handle the bus communication.
pub async fn bus_loop() {
    tokio::spawn(cpu_ram::gather_sysinfo());
    main_loop_wrapper().await;
}

async fn main_loop_wrapper() {
    let loop_result = main_loop().await;
    if let Err(e) = loop_result {
        eprintln!("Error in main loop: {}", e);
        SHOULD_EXIT.store(true, Ordering::Relaxed);
    }
}

async fn main_loop() -> Result<()> {
    // Collection Settings
    let collect_total_throughput = true;
    let collect_top_downloaders = true;

    let mut bus_client = BusClient::new().await?;
    if !bus_client.is_connected() {
        bail!("Failed to connect to the bus");
    }

    loop {
        // Perform actual bus collection
        let mut commands: Vec<BusRequest> = Vec::new();

        if collect_total_throughput {
            commands.push(BusRequest::GetCurrentThroughput);
        }
        if collect_top_downloaders {
            commands.push(BusRequest::GetTopNDownloaders { start: 0, end: 100 });
        }

        // Send the requests and process replies
        for response in bus_client.request(commands).await? {
            match response {
                BusResponse::CurrentThroughput { .. } => throughput::throughput(&response).await,
                BusResponse::TopDownloaders { .. } => top_hosts::top_n(&response).await,
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
