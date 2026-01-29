//! Provides tracking and data-services for per-flow data. Includes implementations
//! of netflow protocols.

mod asn_heatmap;
mod flow_analysis;
mod flow_tracker;
mod netflow5;
mod netflow9;

use crate::throughput_tracker::flow_data::{
    flow_analysis::FinishedFlowAnalysis, netflow5::Netflow5, netflow9::Netflow9,
};
use anyhow::Result;
pub(crate) use asn_heatmap::{AsnAggregate, snapshot_asn_heatmaps, update_asn_heatmaps};
use crossbeam_channel::Sender;
pub(crate) use flow_analysis::{
    AsnCountryListEntry, AsnListEntry, AsnProtocolListEntry, FlowActor, FlowAnalysis,
    RECENT_FLOWS, RttData, expire_rtt_flows, flowbee_handle_events,
    flowbee_rtt_map, get_asn_name_and_country, get_flowbee_event_count_and_reset,
    get_asn_name_by_id, get_rtt_events_per_second, setup_flow_analysis,
    FlowbeeEffectiveDirection, RttBuffer,
};
pub(crate) use flow_tracker::{ALL_FLOWS, AsnId, FlowbeeLocalData};
use lqos_sys::flowbee_data::FlowbeeKey;
use tracing::{debug, error, info};

// Creates the netflow tracker and returns the sender
pub fn setup_netflow_tracker() -> Result<Sender<(FlowbeeKey, (FlowbeeLocalData, FlowAnalysis))>> {
    let (tx, rx) =
        crossbeam_channel::bounded::<(FlowbeeKey, (FlowbeeLocalData, FlowAnalysis))>(65535);
    let config =
        lqos_config::load_config().inspect_err(|e| error!("Failed to load configuration: {e}"))?;

    std::thread::Builder::new()
        .name("Netflow Tracker".to_string())
        .spawn(move || {
            debug!("Starting the network flow tracker back-end");

            // Build the endpoints list
            let mut endpoints: Vec<Sender<(FlowbeeKey, (FlowbeeLocalData, FlowAnalysis))>> =
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
                            let endpoint = Netflow5::new(target)
                                .expect("Cannot parse endpoint for netflow v5");
                            endpoints.push(endpoint);
                            info!("Netflow 5 endpoint added");
                        }
                        9 => {
                            let endpoint = Netflow9::new(target)
                                .expect("Cannot parse endpoint for netflow v9");
                            endpoints.push(endpoint);
                            info!("Netflow 9 endpoint added");
                        }
                        _ => error!("Unsupported netflow version: {version}"),
                    }
                }
            }
            debug!("Flow Endpoints: {}", endpoints.len());

            // Send to all endpoints upon receipt
            while let Ok((key, (value, analysis))) = rx.recv() {
                endpoints.iter_mut().for_each(|f| {
                    //log::debug!("Enqueueing flow data for {key:?}");
                    if let Err(e) = f.try_send((key.clone(), (value.clone(), analysis.clone()))) {
                        tracing::warn!("Failed to send flow data to endpoint: {e}");
                    }
                });
            }
            info!("Network flow tracker back-end has stopped")
        })?;

    Ok(tx)
}
