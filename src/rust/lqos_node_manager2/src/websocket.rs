use axum::extract::WebSocketUpgrade;
use axum::extract::ws::{Message, WebSocket};
use axum::response::IntoResponse;
use serde_json::json;
use tokio::sync::mpsc::Sender;
use lqos_bus::{bus_request, BusRequest, BusResponse, IpStats, TcHandle};
use std::sync::atomic::Ordering::Relaxed;
use serde::{Deserialize, Serialize};
use crate::tracker::SHAPED_DEVICES;

pub async fn ws_handler(
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    log::info!("WS Upgrade Called");
    ws.on_upgrade(handle_socket)
}

async fn handle_socket(mut socket: WebSocket) {
    log::info!("WebSocket Connected");

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Message>(10);
    loop {
        tokio::select! {
            msg = socket.recv() => {
                match msg {
                    Some(Ok(msg)) => {
                        tokio::spawn(
                            handle_socket_message(msg, tx.clone())
                        );
                    }
                    Some(Err(e)) => {
                        log::error!("Error receiving message: {:?}", e);
                        //break;
                    }
                    None => {
                        log::info!("WebSocket Disconnected");
                        break;
                    }
                }
            },
            msg = rx.recv() => {
                match msg {
                    Some(msg) => {
                        socket.send(msg).await.unwrap();
                    }
                    None => {
                        log::info!("WebSocket Disconnected");
                        break;
                    }
                }
            },
        }
    }
}

async fn handle_socket_message(msg: Message, tx: Sender<Message>) {
    if let Ok(raw) = msg.to_text() {
        let msg = serde_json::from_str::<serde_json::Value>(raw);
        if let Ok(serde_json::Value::Object(msg)) = msg {
            let verb = msg.get("type").unwrap().as_str().unwrap();
            match verb {
                "hello" => {
                    log::info!("Received initial hello message");
                    handle_hello(tx.clone()).await
                }
                "flowcount" => flow_counter(tx.clone()).await,
                "shapeddevicecount" => shaped_device_counter(tx.clone()).await,
                "throughput" => throughput_counter(tx.clone()).await,
                "throughputFull" => throughput_full(tx.clone()).await,
                "rttHisto" => rtt_histo(tx.clone()).await,
                "networkTreeSummary" => network_tree_summary(tx.clone()).await,
                "top10Downloaders" => top10downloaders(tx.clone()).await,
                _ => {
                    log::warn!("Unknown WSS verb requested: {verb}");
                }
            }
        } else {
            log::warn!("Unable to decode incoming WSS data: {raw}");
        }
    }
}

async fn handle_hello(tx: Sender<Message>) {
    let response = json!(
        { "type" : "Ack" }
    );
    tx.send(Message::Text(response.to_string())).await.unwrap();
}

async fn flow_counter(tx: Sender<Message>) {
    let response = json!(
        {
            "type" : "FlowCount",
            "count" : crate::FLOW_COUNT.load(Relaxed)
        }
    );
    tx.send(Message::Text(response.to_string())).await.unwrap();
}

async fn shaped_device_counter(tx: Sender<Message>) {
    let response = json!(
        {
            "type" : "ShapedDeviceCount",
            "count" : crate::SHAPED_DEVICE_COUNT.load(Relaxed)
        }
    );
    tx.send(Message::Text(response.to_string())).await.unwrap();
}

async fn throughput_counter(tx: Sender<Message>) {
    let response = json!(
        {
            "type" : "Throughput",
            "bps" : [ crate::TOTAL_BITS_PER_SECOND.0.load(Relaxed), crate::TOTAL_BITS_PER_SECOND.1.load(Relaxed) ],
            "shaped" : [ crate::SHAPED_BITS_PER_SECOND.0.load(Relaxed), crate::SHAPED_BITS_PER_SECOND.1.load(Relaxed) ],
            "pps" : [ crate::PACKETS_PER_SECOND.0.load(Relaxed), crate::PACKETS_PER_SECOND.1.load(Relaxed) ],
        }
    );
    tx.send(Message::Text(response.to_string())).await.unwrap();
}

async fn throughput_full(tx: Sender<Message>) {
    let ring_buffer = {
        let lock = crate::tracker::THROUGHPUT_RING_BUFFER.lock().unwrap();
        lock.fetch()
    };
    let response = json!(
        {
            "type" : "ThroughputFull",
            "entries" : ring_buffer,
        }
    );
    tx.send(Message::Text(response.to_string())).await.unwrap();
}

async fn rtt_histo(tx: Sender<Message>) {
    if let Ok(messages) = bus_request(vec![BusRequest::RttHistogram]).await
    {
        for msg in messages {
            if let BusResponse::RttHistogram(stats) = msg {
                let response = json!(
                  {
                      "type" : "RttHisto",
                      "entries" : stats,
                  }
                );
                tx.send(Message::Text(response.to_string())).await.unwrap();
            }
        }
    }
}

async fn network_tree_summary(tx: Sender<Message>) {
    let responses =
        bus_request(vec![BusRequest::TopMapQueues(4)]).await.unwrap();
    let result = match &responses[0] {
        BusResponse::NetworkMap(nodes) => nodes.to_owned(),
        _ => Vec::new(),
    };
    let response = json!(
                  {
                      "type" : "NetworkTreeSummary",
                      "entries" : result,
                  }
                );
    tx.send(Message::Text(response.to_string())).await.unwrap();
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct IpStatsWithPlan {
    pub ip_address: String,
    pub bits_per_second: (u64, u64),
    pub packets_per_second: (u64, u64),
    pub median_tcp_rtt: f32,
    pub tc_handle: TcHandle,
    pub circuit_id: String,
    pub plan: (u32, u32),
    pub tcp_retransmits: (u64, u64),
}

impl From<&IpStats> for IpStatsWithPlan {
    fn from(i: &IpStats) -> Self {
        let mut result = Self {
            ip_address: i.ip_address.clone(),
            bits_per_second: i.bits_per_second,
            packets_per_second: i.packets_per_second,
            median_tcp_rtt: i.median_tcp_rtt,
            tc_handle: i.tc_handle,
            circuit_id: i.circuit_id.clone(),
            plan: (0, 0),
            tcp_retransmits: i.tcp_retransmits,
        };

        if !result.circuit_id.is_empty() {
            if let Some(circuit) = SHAPED_DEVICES
                .read()
                .unwrap()
                .devices
                .iter()
                .find(|sd| sd.circuit_id == result.circuit_id)
            {
                let name = if circuit.circuit_name.len() > 20 {
                    &circuit.circuit_name[0..20]
                } else {
                    &circuit.circuit_name
                };
                result.ip_address = format!("{} ({})", name, result.ip_address);
                result.plan = (circuit.download_max_mbps, circuit.upload_max_mbps);
            }
        }

        result
    }
}

async fn top10downloaders(tx: Sender<Message>) {
    if let Ok(messages) = bus_request(vec![BusRequest::GetTopNDownloaders { start: 0, end: 10 }]).await
    {
        for msg in messages {
            if let BusResponse::TopDownloaders(stats) = msg {
                let result: Vec<IpStatsWithPlan> = stats.iter().map(|tt| tt.into()).collect();
                let response = json!(
                    {
                        "type" : "Top10Downloaders",
                        "entries" : result,
                    }
                );
                tx.send(Message::Text(response.to_string())).await.unwrap();
            }
        }
    }
}