use crate::web::wss::{
    nodes::node_status,
    queries::{
        ext_device::{
            send_extended_device_capacity_graph, send_extended_device_info,
            send_extended_device_snr_graph,
        },
        omnisearch, root_heat_map, send_circuit_info, send_packets_for_all_nodes,
        send_packets_for_node, send_perf_for_node, send_rtt_for_all_nodes,
        send_rtt_for_all_nodes_circuit, send_rtt_for_all_nodes_site, send_rtt_for_node,
        send_site_info, send_site_parents, send_site_stack_map, send_throughput_for_all_nodes,
        send_throughput_for_all_nodes_by_circuit, send_throughput_for_all_nodes_by_site,
        send_throughput_for_node, site_heat_map,
        site_tree::send_site_tree,
        time_period::InfluxTimePeriod,
        send_circuit_parents, send_root_parents,
    },
};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use pgdb::sqlx::{Pool, Postgres};
use wasm_pipe_types::{WasmRequest, WasmResponse};
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
    tracing::info!("WebSocket Connected");
    let mut credentials: Option<login::LoginResult> = None;
    while let Some(msg) = socket.recv().await {
        let cnn = cnn.clone();
        let msg = msg.unwrap();

        // Get the binary message and decompress it
        tracing::info!("Received a message: {:?}", msg);
        let raw = msg.into_data();
        let uncompressed = miniz_oxide::inflate::decompress_to_vec(&raw).unwrap();
        let msg = lts_client::cbor::from_slice::<WasmRequest>(&uncompressed).unwrap();
        tracing::info!("{msg:?}");

        // Update the token credentials (if there are any)
        if let Some(credentials) = &credentials {
            let _ = pgdb::refresh_token(cnn.clone(), &credentials.token).await;
        }

        // Handle the message by type
        let matcher = (&msg, &mut credentials);
        let wss = &mut socket;
        match matcher {
            // Handle login with just a token
            (WasmRequest::Auth { token }, _) => {
                let result = login::on_token_auth(token, &mut socket, cnn).await;
                if let Some(result) = result {
                    credentials = Some(result);
                }
            }
            // Handle login with a username and password
            (
                WasmRequest::Login {
                    license,
                    username,
                    password,
                },
                _,
            ) => {
                let result = login::on_login(license, username, password, &mut socket, cnn).await;
                if let Some(result) = result {
                    credentials = Some(result);
                }
            }
            // Node status for dashboard
            (WasmRequest::GetNodeStatus, Some(credentials)) => {
                node_status(&cnn, wss, &credentials.license_key).await;
            }
            // Packet chart for dashboard
            (WasmRequest::PacketChart { period }, Some(credentials)) => {
                let _ =
                    send_packets_for_all_nodes(&cnn, wss, &credentials.license_key, period.into())
                        .await;
            }
            // Packet chart for individual node
            (
                WasmRequest::PacketChartSingle {
                    period,
                    node_id,
                    node_name,
                },
                Some(credentials),
            ) => {
                let _ = send_packets_for_node(
                    &cnn,
                    wss,
                    &credentials.license_key,
                    period.into(),
                    node_id,
                    node_name,
                )
                .await;
            }
            // Throughput chart for the dashboard
            (WasmRequest::ThroughputChart { period }, Some(credentials)) => {
                let _ = send_throughput_for_all_nodes(
                    &cnn,
                    wss,
                    &credentials.license_key,
                    InfluxTimePeriod::new(period),
                )
                .await;
            }
            // Throughput chart for a single shaper node
            (
                WasmRequest::ThroughputChartSingle {
                    period,
                    node_id,
                    node_name,
                },
                Some(credentials),
            ) => {
                let _ = send_throughput_for_node(
                    &cnn,
                    wss,
                    &credentials.license_key,
                    InfluxTimePeriod::new(period),
                    node_id.to_string(),
                    node_name.to_string(),
                )
                .await;
            }
            (WasmRequest::ThroughputChartSite { period, site_id }, Some(credentials)) => {
                let _ = send_throughput_for_all_nodes_by_site(
                    &cnn,
                    wss,
                    &credentials.license_key,
                    site_id.to_string(),
                    InfluxTimePeriod::new(period),
                )
                .await;
            }
            (WasmRequest::ThroughputChartCircuit { period, circuit_id }, Some(credentials)) => {
                let _ = send_throughput_for_all_nodes_by_circuit(
                    &cnn,
                    wss,
                    &credentials.license_key,
                    circuit_id.to_string(),
                    InfluxTimePeriod::new(period),
                )
                .await;
            }
            // Rtt Chart
            (WasmRequest::RttChart { period }, Some(credentials)) => {
                let _ = send_rtt_for_all_nodes(
                    &cnn,
                    wss,
                    &credentials.license_key,
                    InfluxTimePeriod::new(period),
                )
                .await;
            }
            (WasmRequest::RttChartSite { period, site_id }, Some(credentials)) => {
                let _ = send_rtt_for_all_nodes_site(
                    &cnn,
                    wss,
                    &credentials.license_key,
                    site_id.to_string(),
                    InfluxTimePeriod::new(period),
                )
                .await;
            }
            (
                WasmRequest::RttChartSingle {
                    period,
                    node_id,
                    node_name,
                },
                Some(credentials),
            ) => {
                let _ = send_rtt_for_node(
                    &cnn,
                    wss,
                    &credentials.license_key,
                    InfluxTimePeriod::new(period),
                    node_id.to_string(),
                    node_name.to_string(),
                )
                .await;
            }
            (WasmRequest::RttChartCircuit { period, circuit_id }, Some(credentials)) => {
                let _ = send_rtt_for_all_nodes_circuit(
                    &cnn,
                    wss,
                    &credentials.license_key,
                    circuit_id.to_string(),
                    InfluxTimePeriod::new(period),
                )
                .await;
            }
            // Site Stack
            (WasmRequest::SiteStack { period, site_id }, Some(credentials)) => {
                let _ = send_site_stack_map(
                    &cnn,
                    wss,
                    &credentials.license_key,
                    InfluxTimePeriod::new(period),
                    site_id.to_string(),
                )
                .await;
            }
            (WasmRequest::RootHeat { period }, Some(credentials)) => {
                let _ = root_heat_map(
                    &cnn,
                    wss,
                    &credentials.license_key,
                    InfluxTimePeriod::new(period),
                )
                .await;
            }
            (WasmRequest::SiteHeat { period, site_id }, Some(credentials)) => {
                let _ = site_heat_map(
                    &cnn,
                    wss,
                    &credentials.license_key,
                    site_id,
                    InfluxTimePeriod::new(period),
                )
                .await;
            }
            (
                WasmRequest::NodePerfChart {
                    period,
                    node_id,
                    node_name,
                },
                Some(credentials),
            ) => {
                let _ = send_perf_for_node(
                    &cnn,
                    wss,
                    &credentials.license_key,
                    InfluxTimePeriod::new(period),
                    node_id.to_string(),
                    node_name.to_string(),
                )
                .await;
            }
            (WasmRequest::Tree { parent }, Some(credentials)) => {
                send_site_tree(&cnn, wss, &credentials.license_key, parent).await;
            }
            (WasmRequest::SiteInfo { site_id }, Some(credentials)) => {
                send_site_info(&cnn, wss, &credentials.license_key, site_id).await;
            }
            (WasmRequest::SiteParents { site_id }, Some(credentials)) => {
                send_site_parents(&cnn, wss, &credentials.license_key, site_id).await;
            }
            (WasmRequest::CircuitParents { circuit_id }, Some(credentials)) => {
                send_circuit_parents(&cnn, wss, &credentials.license_key, circuit_id).await;
            }
            (WasmRequest::RootParents, Some(credentials)) => {
                send_root_parents(&cnn, wss, &credentials.license_key).await;
            }
            (WasmRequest::Search { term }, Some(credentials)) => {
                let _ = omnisearch(&cnn, wss, &credentials.license_key, term).await;
            }
            (WasmRequest::CircuitInfo { circuit_id }, Some(credentials)) => {
                send_circuit_info(&cnn, wss, &credentials.license_key, circuit_id).await;
            }
            (WasmRequest::ExtendedDeviceInfo { circuit_id }, Some(credentials)) => {
                send_extended_device_info(&cnn, wss, &credentials.license_key, circuit_id).await;
            }
            (WasmRequest::SignalNoiseChartExt { period, device_id }, Some(credentials)) => {
                let _ = send_extended_device_snr_graph(
                    &cnn,
                    wss,
                    &credentials.license_key,
                    device_id,
                    InfluxTimePeriod::new(period),
                )
                .await;
            }
            (WasmRequest::DeviceCapacityChartExt { period, device_id }, Some(credentials)) => {
                let _ = send_extended_device_capacity_graph(
                    &cnn,
                    wss,
                    &credentials.license_key,
                    device_id,
                    InfluxTimePeriod::new(period),
                )
                .await;
            }
            (_, None) => {
                tracing::error!("No credentials");
            }
        }
    }
}

fn serialize_response(response: WasmResponse) -> Vec<u8> {
    let cbor = lts_client::cbor::to_vec(&response).unwrap();
    miniz_oxide::deflate::compress_to_vec(&cbor, 8)
}

pub async fn send_response(socket: &mut WebSocket, response: WasmResponse) {
    let serialized = serialize_response(response);
    socket.send(Message::Binary(serialized)).await.unwrap();
}
