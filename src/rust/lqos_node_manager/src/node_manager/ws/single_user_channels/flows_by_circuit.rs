use std::net::IpAddr;
use std::time::Duration;
use serde::Serialize;
use tokio::time::MissedTickBehavior;
use tracing::debug;
use lqos_bus::{bus_request, BusRequest, BusResponse, FlowAnalysisTransport, FlowbeeKeyTransit, FlowbeeLocalData};
use lqos_utils::unix_time::time_since_boot;
use crate::shaped_devices_tracker::SHAPED_DEVICES;

#[derive(Serialize)]
struct FlowData {
    circuit_id: String,
    flows: Vec<(FlowbeeKeyTransit, FlowbeeLocalData, FlowAnalysisTransport)>,
}

pub(super) async fn flows_by_circuit(circuit: String, tx: tokio::sync::mpsc::Sender<String>) {
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        let Ok(replies) = bus_request(vec![BusRequest::FlowsByCircuit(circuit.clone())]).await else {
            continue;
        };
        for reply in replies {
            if let BusResponse::FlowsByCircuit(flows) = reply {
                if !flows.is_empty() {
                    let result = FlowData {
                        circuit_id: circuit.clone(),
                        flows,
                    };
                    if let Ok(message) = serde_json::to_string(&result) {
                        if let Err(_) = tx.send(message).await {
                            debug!("Channel is gone");
                            break;
                        }
                    }
                }
            }
        }

        ticker.tick().await;
    }
}