//! Provides tracking and data-services for per-flow data. Includes implementations
//! of netflow protocols.

mod flow_tracker;
mod netflow5;
mod netflow9;

use crate::throughput_tracker::flow_data::{netflow5::Netflow5, netflow9::Netflow9};
pub(crate) use flow_tracker::ALL_FLOWS;
use lqos_sys::flowbee_data::{FlowbeeData, FlowbeeKey};
use std::sync::{
    mpsc::{channel, Sender},
    Arc,
};

trait FlowbeeRecipient {
    fn enqueue(&self, key: FlowbeeKey, data: FlowbeeData);
}

// Creates the netflow tracker and returns the sender
pub fn setup_netflow_tracker() -> Sender<(FlowbeeKey, FlowbeeData)> {
    let (tx, rx) = channel::<(FlowbeeKey, FlowbeeData)>();
    let config = lqos_config::load_config().unwrap();

    std::thread::spawn(move || {
        log::info!("Starting the network flow tracker back-end");

        // Build the endpoints list
        let mut endpoints: Vec<Arc<dyn FlowbeeRecipient>> = Vec::new();
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

        // Send to all endpoints upon receipt
        while let Ok((key, value)) = rx.recv() {
            endpoints.iter_mut().for_each(|f| {
                log::debug!("Enqueueing flow data for {key:?}");
                f.enqueue(key.clone(), value.clone());
            });
        }
        log::info!("Network flow tracker back-end has stopped")
    });

    tx
}
