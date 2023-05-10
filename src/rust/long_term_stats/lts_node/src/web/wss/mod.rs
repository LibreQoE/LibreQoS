use crate::web::wss::queries::{
    omnisearch, root_heat_map, send_packets_for_all_nodes, send_packets_for_node,
    send_perf_for_node, send_rtt_for_all_nodes, send_rtt_for_node, send_throughput_for_all_nodes,
    send_throughput_for_node, site_tree::send_site_tree, send_throughput_for_all_nodes_by_site, send_site_info, send_rtt_for_all_nodes_site,
};
use axum::{
    extract::{
        ws::{WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use pgdb::sqlx::{Pool, Postgres};
use serde_json::Value;
mod login;
mod nodes;
mod queries;

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Pool<Postgres>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |sock| handle_socket(sock, state))
}

async fn handle_socket(mut socket: WebSocket, cnn: Pool<Postgres>) {
    log::info!("WebSocket Connected");
    let mut credentials: Option<login::LoginResult> = None;
    while let Some(msg) = socket.recv().await {
        let cnn = cnn.clone();
        let msg = msg.unwrap();
        log::info!("Received a message: {:?}", msg);
        if let Ok(text) = msg.into_text() {
            let json = serde_json::from_str::<Value>(&text);
            if json.is_err() {
                log::warn!("Unable to parse JSON: {}", json.err().unwrap());
            } else if let Ok(json) = json {
                log::info!("Received a JSON: {:?}", json);

                if let Some(credentials) = &credentials {
                    let _ = pgdb::refresh_token(cnn.clone(), &credentials.token).await;
                }

                let period =
                    queries::time_period::InfluxTimePeriod::new(json.get("period").cloned());

                if let Some(Value::String(msg_type)) = json.get("msg") {
                    match msg_type.as_str() {
                        "login" => {
                            // A full login request
                            let result = login::on_login(&json, &mut socket, cnn).await;
                            if let Some(result) = result {
                                credentials = Some(result);
                            }
                        }
                        "auth" => {
                            // Login with just a token
                            let result = login::on_token_auth(&json, &mut socket, cnn).await;
                            if let Some(result) = result {
                                credentials = Some(result);
                            }
                        }
                        "nodeStatus" => {
                            if let Some(credentials) = &credentials {
                                nodes::node_status(
                                    cnn.clone(),
                                    &mut socket,
                                    &credentials.license_key,
                                )
                                .await;
                            } else {
                                log::info!("Node status requested but no credentials provided");
                            }
                        }
                        "packetChart" => {
                            if let Some(credentials) = &credentials {
                                let _ = send_packets_for_all_nodes(
                                    cnn.clone(),
                                    &mut socket,
                                    &credentials.license_key,
                                    period,
                                )
                                .await;
                            } else {
                                log::info!("Throughput requested but no credentials provided");
                            }
                        }
                        "packetChartSingle" => {
                            if let Some(credentials) = &credentials {
                                let _ = send_packets_for_node(
                                    cnn.clone(),
                                    &mut socket,
                                    &credentials.license_key,
                                    period,
                                    json.get("node_id").unwrap().as_str().unwrap().to_string(),
                                    json.get("node_name").unwrap().as_str().unwrap().to_string(),
                                )
                                .await;
                            } else {
                                log::info!("Throughput requested but no credentials provided");
                            }
                        }
                        "throughputChart" => {
                            if let Some(credentials) = &credentials {
                                let _ = send_throughput_for_all_nodes(
                                    cnn.clone(),
                                    &mut socket,
                                    &credentials.license_key,
                                    period,
                                )
                                .await;
                            } else {
                                log::info!("Throughput requested but no credentials provided");
                            }
                        }
                        "throughputChartSite" => {
                            if let Some(credentials) = &credentials {
                                let _ = send_throughput_for_all_nodes_by_site(
                                    cnn.clone(),
                                    &mut socket,
                                    &credentials.license_key,
                                    json.get("site_id").unwrap().as_str().unwrap().to_string(),
                                    period,
                                )
                                .await;
                            } else {
                                log::info!("Throughput requested but no credentials provided");
                            }
                        }
                        "throughputChartSingle" => {
                            if let Some(credentials) = &credentials {
                                let _ = send_throughput_for_node(
                                    cnn.clone(),
                                    &mut socket,
                                    &credentials.license_key,
                                    period,
                                    json.get("node_id").unwrap().as_str().unwrap().to_string(),
                                    json.get("node_name").unwrap().as_str().unwrap().to_string(),
                                )
                                .await;
                            } else {
                                log::info!("Throughput requested but no credentials provided");
                            }
                        }
                        "rttChart" => {
                            if let Some(credentials) = &credentials {
                                let _ = send_rtt_for_all_nodes(
                                    cnn.clone(),
                                    &mut socket,
                                    &credentials.license_key,
                                    period,
                                )
                                .await;
                            } else {
                                log::info!("Throughput requested but no credentials provided");
                            }
                        }
                        "rttChartSite" => {
                            if let Some(credentials) = &credentials {
                                let _ = send_rtt_for_all_nodes_site(
                                    cnn.clone(),
                                    &mut socket,
                                    &credentials.license_key,
                                    json.get("site_id").unwrap().as_str().unwrap().to_string(),
                                    period,
                                )
                                .await;
                            } else {
                                log::info!("Throughput requested but no credentials provided");
                            }
                        }
                        "rttChartSingle" => {
                            if let Some(credentials) = &credentials {
                                let _ = send_rtt_for_node(
                                    cnn.clone(),
                                    &mut socket,
                                    &credentials.license_key,
                                    period,
                                    json.get("node_id").unwrap().as_str().unwrap().to_string(),
                                    json.get("node_name").unwrap().as_str().unwrap().to_string(),
                                )
                                .await;
                            } else {
                                log::info!("Throughput requested but no credentials provided");
                            }
                        }
                        "nodePerf" => {
                            if let Some(credentials) = &credentials {
                                let _ = send_perf_for_node(
                                    cnn.clone(),
                                    &mut socket,
                                    &credentials.license_key,
                                    period,
                                    json.get("node_id").unwrap().as_str().unwrap().to_string(),
                                    json.get("node_name").unwrap().as_str().unwrap().to_string(),
                                )
                                .await;
                            } else {
                                log::info!("Throughput requested but no credentials provided");
                            }
                        }
                        "search" => {
                            if let Some(credentials) = &credentials {
                                let _ = omnisearch(
                                    cnn.clone(),
                                    &mut socket,
                                    &credentials.license_key,
                                    json.get("term").unwrap().as_str().unwrap(),
                                )
                                .await;
                            }
                        }
                        "siteRootHeat" => {
                            if let Some(credentials) = &credentials {
                                let _ = root_heat_map(
                                    cnn.clone(),
                                    &mut socket,
                                    &credentials.license_key,
                                    period,
                                )
                                .await;
                            } else {
                                log::info!("Throughput requested but no credentials provided");
                            }
                        }
                        "siteTree" => {
                            if let Some(credentials) = &credentials {
                                send_site_tree(
                                    cnn.clone(),
                                    &mut socket,
                                    &credentials.license_key,
                                    json.get("parent").unwrap().as_str().unwrap(),
                                )
                                .await;
                            }
                        }
                        "siteInfo" => {
                            if let Some(credentials) = &credentials {
                                send_site_info(
                                    cnn.clone(),
                                    &mut socket,
                                    &credentials.license_key,
                                    json.get("site_id").unwrap().as_str().unwrap(),
                                )
                                .await;
                            }
                        }
                        _ => {
                            log::warn!("Unknown message type: {msg_type}");
                        }
                    }
                }
            }
        }
    }
}
