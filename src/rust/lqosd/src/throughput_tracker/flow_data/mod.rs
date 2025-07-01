//! Provides tracking and data-services for per-flow data. Includes implementations
//! of netflow protocols.

mod flow_analysis;
mod flow_tracker;
mod netflow5;
mod netflow9;

use crate::throughput_tracker::flow_data::{
    flow_analysis::FinishedFlowAnalysis, netflow5::Netflow5, netflow9::Netflow9,
};
use anyhow::Result;
use crossbeam_channel::Sender;
pub(crate) use flow_analysis::{
    AsnCountryListEntry, AsnListEntry, AsnProtocolListEntry, FlowActor, FlowAnalysis,
    FlowAnalysisSystem, RECENT_FLOWS, RttData, expire_rtt_flows, flowbee_handle_events,
    flowbee_rtt_map, get_asn_name_and_country, get_flowbee_event_count_and_reset,
    get_rtt_events_per_second, setup_flow_analysis,
};
pub(crate) use flow_tracker::{ALL_FLOWS, AsnId, FlowbeeLocalData, MAX_RETRY_TIMESTAMPS};
use lqos_sys::flowbee_data::FlowbeeKey;
use tracing::{debug, error, info};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Maximum capacity for flow data channels
pub const FLOW_CHANNEL_CAPACITY: usize = 65535;

/// Counter for flows discarded due to full netflow channel
static DISCARDED_NETFLOW_FLOWS: AtomicU64 = AtomicU64::new(0);

// Creates the netflow tracker and returns the sender
pub fn setup_netflow_tracker() -> Result<Sender<(FlowbeeKey, Arc<Mutex<(FlowbeeLocalData, FlowAnalysis)>>)>> {
    let (tx, rx) =
        crossbeam_channel::bounded::<(FlowbeeKey, Arc<Mutex<(FlowbeeLocalData, FlowAnalysis)>>)>(FLOW_CHANNEL_CAPACITY);
    let config =
        lqos_config::load_config().inspect_err(|e| error!("Failed to load configuration: {e}"))?;

    std::thread::Builder::new()
        .name("Netflow Tracker".to_string())
        .spawn(move || {
            debug!("Starting the network flow tracker back-end");

            // Build the endpoints list
            let mut endpoints: Vec<Sender<(FlowbeeKey, Arc<Mutex<(FlowbeeLocalData, FlowAnalysis)>>)>> =
                Vec::new();
            endpoints.push(FinishedFlowAnalysis::new());

            if let Some(flow_config) = &config.flows {
                if let (Some(ip), Some(port), Some(version)) = (
                    flow_config.netflow_ip.clone(),
                    flow_config.netflow_port,
                    flow_config.netflow_version,
                ) {
                    info!("Setting up netflow target: {ip}:{port}, version: {version}");
                    let target = format!("{ip}:{port}", ip = ip, port = port);
                    match version {
                        5 => {
                            let endpoint = Netflow5::new(target).unwrap();
                            endpoints.push(endpoint);
                            info!("Netflow 5 endpoint added");
                        }
                        9 => {
                            let endpoint = Netflow9::new(target).unwrap();
                            endpoints.push(endpoint);
                            info!("Netflow 9 endpoint added");
                        }
                        _ => error!("Unsupported netflow version: {version}"),
                    }
                }
            }
            debug!("Flow Endpoints: {}", endpoints.len());

            // Send to all endpoints upon receipt
            while let Ok((key, value)) = rx.recv() {
                endpoints.iter_mut().for_each(|f| {
                    //log::debug!("Enqueueing flow data for {key:?}");
                    if f.try_send((key.clone(), value.clone())).is_err() {
                        DISCARDED_NETFLOW_FLOWS.fetch_add(1, Ordering::Relaxed);
                        tracing::warn!("Failed to send flow data to endpoint: channel full");
                    }
                });
            }
            info!("Network flow tracker back-end has stopped")
        })?;

    Ok(tx)
}
