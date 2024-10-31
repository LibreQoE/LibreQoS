//! Handles the communication loop with lqosd.

use crate::ui_base::SHOULD_EXIT;
use anyhow::{bail, Result};
use lqos_bus::{BusClient, BusRequest, BusResponse};
use tokio::sync::mpsc::Receiver;
use std::sync::atomic::Ordering;
pub mod cpu_ram;

/// Communications with the bus via channels
pub enum BusMessage {
    EnableTotalThroughput(std::sync::mpsc::Sender<BusResponse>),
    DisableTotalThroughput,
    EnableTopFlows(std::sync::mpsc::Sender<BusResponse>),
    DisableTopFlows,
    EnableTopHosts(std::sync::mpsc::Sender<BusResponse>),
    DisableTopHosts,
    EnableLatencyHistogram(std::sync::mpsc::Sender<BusResponse>),
    DisableLatencyHistogram,
}

/// The main loop for the bus.
/// Spawns a separate task to handle the bus communication.
pub async fn bus_loop(rx: Receiver<BusMessage>) {
    tokio::spawn(cpu_ram::gather_sysinfo());
    main_loop_wrapper(rx).await;
}

async fn main_loop_wrapper(rx: Receiver<BusMessage>) {
    let loop_result = main_loop(rx).await;
    if let Err(e) = loop_result {
        SHOULD_EXIT.store(true, Ordering::Relaxed);
        panic!("Error in main loop: {}", e);
    }
}

async fn main_loop(mut rx: Receiver<BusMessage>) -> Result<()> {
    // Collection Settings
    let mut collect_total_throughput = None;
    let mut collect_top_downloaders = None;
    let mut collect_top_flows = None;
    let mut collect_latency_histogram = None;

    let mut bus_client = BusClient::new().await?;
    if !bus_client.is_connected() {
        bail!("Failed to connect to the bus");
    }

    loop {
        // See if there are any messages
        while let Ok(msg) = rx.try_recv() {
            match msg {
                BusMessage::EnableTotalThroughput(tx) => {
                    collect_total_throughput = Some(tx);
                }
                BusMessage::DisableTotalThroughput => {
                    collect_total_throughput = None;
                }
                BusMessage::EnableTopFlows(tx) => {
                    collect_top_flows = Some(tx);
                }
                BusMessage::DisableTopFlows => {
                    collect_top_flows = None;
                }
                BusMessage::EnableTopHosts(tx) => {
                    collect_top_downloaders = Some(tx);
                }
                BusMessage::DisableTopHosts => {
                    collect_top_downloaders = None;
                }
                BusMessage::EnableLatencyHistogram(tx) => {
                    collect_latency_histogram = Some(tx);
                }
                BusMessage::DisableLatencyHistogram => {
                    collect_latency_histogram = None;
                }
            }
        }

        // Perform actual bus collection
        let mut commands: Vec<BusRequest> = Vec::new();

        if collect_total_throughput.is_some() {
            commands.push(BusRequest::GetCurrentThroughput);
        }
        if collect_top_downloaders.is_some() {
            commands.push(BusRequest::GetTopNDownloaders { start: 0, end: 100 });
        }
        if collect_top_flows.is_some() {
            commands.push(BusRequest::TopFlows { flow_type: lqos_bus::TopFlowType::Bytes, n: 100 });
        }
        if collect_latency_histogram.is_some() {
            commands.push(BusRequest::RttHistogram);
        }

        // Send the requests and process replies
        for response in bus_client.request(commands).await? {
            match response {
                BusResponse::CurrentThroughput { .. } => {
                    if let Some(tx) = &collect_total_throughput {
                        let _ = tx.send(response); // Ignoring the error, it's ok if the channel closed
                    }
                }
                BusResponse::TopDownloaders { .. } => {
                    if let Some(tx) = &collect_top_downloaders {
                        let _ = tx.send(response); // Ignoring the error, it's ok if the channel closed
                    }
                }
                BusResponse::TopFlows(..) => {
                    if let Some(tx) = &collect_top_flows {
                        let _ = tx.send(response); // Ignoring the error, it's ok if the channel closed
                    }
                }
                BusResponse::RttHistogram(..) => {
                    if let Some(tx) = &collect_latency_histogram {
                        let _ = tx.send(response); // Ignoring the error, it's ok if the channel closed
                    }
                }
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
