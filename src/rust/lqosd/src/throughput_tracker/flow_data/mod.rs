//! Provides tracking and data-services for per-flow data. Includes implementations
//! of netflow protocols.

mod flow_tracker;
mod netflow5;
mod netflow9;
mod flow_analysis;

use crate::throughput_tracker::flow_data::{flow_analysis::FinishedFlowAnalysis, netflow5::Netflow5, netflow9::Netflow9};
pub(crate) use flow_tracker::{ALL_FLOWS, AsnId, FlowbeeLocalData};
use lqos_sys::flowbee_data::FlowbeeKey;
use std::sync::{
    mpsc::{channel, Sender},
    Arc,
};
pub(crate) use flow_analysis::{setup_flow_analysis, get_asn_name_and_country, 
    FlowAnalysis, RECENT_FLOWS, flowbee_handle_events, get_flowbee_event_count_and_reset,
    expire_rtt_flows, flowbee_rtt_map, RttData, get_rtt_events_per_second,
};


trait FlowbeeRecipient {
    fn enqueue(&self, key: FlowbeeKey, data: FlowbeeLocalData, analysis: FlowAnalysis);
}

// Creates the netflow tracker and returns the sender
pub fn setup_netflow_tracker() -> Sender<(FlowbeeKey, (FlowbeeLocalData, FlowAnalysis))> {
    let (tx, rx) = channel::<(FlowbeeKey, (FlowbeeLocalData, FlowAnalysis))>();
    let config = lqos_config::load_config().unwrap();

    std::thread::spawn(move || {
        log::info!("Starting the network flow tracker back-end");

        // Build the endpoints list
        let mut endpoints: Vec<Arc<dyn FlowbeeRecipient>> = Vec::new();
        endpoints.push(FinishedFlowAnalysis::new());

        if let Some(flow_config) = config.flows {
            if let (Some(ip), Some(port), Some(version)) = (
                flow_config.netflow_ip,
                flow_config.netflow_port,
                flow_config.netflow_version,
            ) {
                log::info!("Setting up netflow target: {ip}:{port}, version: {version}");
                let target = format!("{ip}:{port}", ip = ip, port = port);
                match version {
                    5 => {
                        let endpoint = Netflow5::new(target).unwrap();
                        endpoints.push(endpoint);
                        log::info!("Netflow 5 endpoint added");
                    }
                    9 => {
                        let endpoint = Netflow9::new(target).unwrap();
                        endpoints.push(endpoint);
                        log::info!("Netflow 9 endpoint added");
                    }
                    _ => log::error!("Unsupported netflow version: {version}"),
                }
            }
        }
        log::info!("Flow Endpoints: {}", endpoints.len());

        // Send to all endpoints upon receipt
        while let Ok((key, (value, analysis))) = rx.recv() {
            endpoints.iter_mut().for_each(|f| {
                //log::debug!("Enqueueing flow data for {key:?}");
                f.enqueue(key.clone(), value.clone(), analysis.clone());
            });
        }
        log::info!("Network flow tracker back-end has stopped")
    });

    tx
}
