use std::sync::Arc;
use crate::web::wss::{
    nodes::node_status,
    queries::{
        ext_device::{
            send_extended_device_capacity_graph, send_extended_device_info,
            send_extended_device_snr_graph,
        },
        omnisearch, root_heat_map, send_circuit_info, send_circuit_parents,
        send_packets_for_all_nodes, send_packets_for_node, send_perf_for_node, send_root_parents,
        send_rtt_for_all_nodes, send_rtt_for_all_nodes_circuit, send_rtt_for_all_nodes_site,
        send_rtt_for_node, send_rtt_histogram_for_all_nodes, send_site_info, send_site_parents,
        send_site_stack_map, send_throughput_for_all_nodes,
        send_throughput_for_all_nodes_by_circuit, send_throughput_for_all_nodes_by_site,
        send_throughput_for_node, site_heat_map,
        site_tree::send_site_tree,
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
use tokio::sync::{mpsc::Sender, Mutex};
use tracing::instrument;
use wasm_pipe_types::{WasmRequest, WasmResponse};
use self::queries::InfluxTimePeriod;
mod login;
mod nodes;
mod queries;

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Pool<Postgres>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |sock| handle_socket(sock, state))
}

#[instrument(skip(socket, cnn), name = "handle_wss")]
async fn handle_socket(mut socket: WebSocket, cnn: Pool<Postgres>) {
    tracing::info!("WebSocket Connected");
    let credentials: Arc<Mutex<Option<login::LoginResult>>> = Arc::new(Mutex::new(None));

    // Setup the send/receive channel
    let (tx, mut rx) = tokio::sync::mpsc::channel::<WasmResponse>(10);

    loop {
        tokio::select! {
            msg = socket.recv() => {
                match msg {
                    Some(msg) => {
                        tokio::spawn(
                            handle_socket_message(msg.unwrap(), cnn.clone(), credentials.clone(), tx.clone())
                        );
                    }
                    None => {
                        tracing::info!("WebSocket Disconnected");
                        break;
                    }
                }
            },
            msg = rx.recv() => {
                match msg {
                    Some(msg) => {
                        let serialized = serialize_response(msg);
                        socket.send(Message::Binary(serialized)).await.unwrap();
                    }
                    None => {
                        tracing::info!("WebSocket Disconnected");
                        break;
                    }
                }
            },
        }
    }
}

#[instrument(skip(credentials, cnn))]
async fn update_token_credentials(
    credentials: Arc<Mutex<Option<login::LoginResult>>>,
    cnn: Pool<Postgres>,
) {
    let mut credentials = credentials.lock().await;
    if let Some(credentials) = &mut *credentials {
        let _ = pgdb::refresh_token(cnn, &credentials.token).await;
    }
}

async fn set_credentials(
    credentials: Arc<Mutex<Option<login::LoginResult>>>,
    result: login::LoginResult,
) {
    let mut credentials = credentials.lock().await;
    *credentials = Some(result);
}

fn extract_message(msg: Message) -> WasmRequest {
    let raw = msg.into_data();
    let uncompressed = miniz_oxide::inflate::decompress_to_vec(&raw).unwrap();
    lts_client::cbor::from_slice::<WasmRequest>(&uncompressed).unwrap()
}

async fn handle_auth_message(
    msg: &WasmRequest, 
    credentials: Arc<Mutex<Option<login::LoginResult>>>,
    tx: Sender<WasmResponse>,
    cnn: Pool<Postgres>,
) {
    match msg {
        // Handle login with just a token
        WasmRequest::Auth { token } => {
            let result = login::on_token_auth(token, tx, cnn).await;
            if let Some(result) = result {
                set_credentials(credentials, result).await;
            }
        }
        // Handle a full login
        WasmRequest::Login { license, username, password } => {
            let result = login::on_login(license, username, password, tx, cnn).await;
            if let Some(result) = result {
                set_credentials(credentials, result).await;
            }
        }
        _ => {}
    }
}

async fn handle_socket_message(
    msg: Message,
    cnn: Pool<Postgres>,
    credentials: Arc<Mutex<Option<login::LoginResult>>>,
    tx: Sender<WasmResponse>,
) {
    // Get the binary message and decompress it
    let msg = extract_message(msg);
    update_token_credentials(credentials.clone(), cnn.clone()).await;

    // Handle the message by type
    handle_auth_message(&msg, credentials.clone(), tx.clone(), cnn.clone()).await;

    let my_credentials = {
        let lock = credentials.lock().await;
        lock.clone()
    };
    let matcher = (&msg, &my_credentials);
    match matcher {
        // Node status for dashboard
        (WasmRequest::GetNodeStatus, Some(credentials)) => {
            node_status(&cnn, tx, &credentials.license_key).await;
        }
        // Packet chart for dashboard
        (WasmRequest::PacketChart { period }, Some(credentials)) => {
            let _ =
                send_packets_for_all_nodes(&cnn, tx, &credentials.license_key, period.into()).await;
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
                tx,
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
                tx,
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
                tx,
                &credentials.license_key,
                InfluxTimePeriod::new(period),
                node_id.to_string(),
                node_name.to_string(),
            )
            .await;
        }
        (WasmRequest::ThroughputChartSite { period, site_id }, Some(credentials)) => {
            let site_id = urlencoding::decode(site_id).unwrap();
            let _ = send_throughput_for_all_nodes_by_site(
                &cnn,
                tx,
                &credentials.license_key,
                site_id.to_string(),
                InfluxTimePeriod::new(period),
            )
            .await;
        }
        (WasmRequest::ThroughputChartCircuit { period, circuit_id }, Some(credentials)) => {
            let _ = send_throughput_for_all_nodes_by_circuit(
                &cnn,
                tx,
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
                tx,
                &credentials.license_key,
                InfluxTimePeriod::new(period),
            )
            .await;
        }
        (WasmRequest::RttHistogram { period }, Some(credentials)) => {
            let _ = send_rtt_histogram_for_all_nodes(
                &cnn,
                tx,
                &credentials.license_key,
                InfluxTimePeriod::new(period),
            )
            .await;
        }
        (WasmRequest::RttChartSite { period, site_id }, Some(credentials)) => {
            let site_id = urlencoding::decode(site_id).unwrap();
            let _ = send_rtt_for_all_nodes_site(
                &cnn,
                tx,
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
                tx,
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
                tx,
                &credentials.license_key,
                circuit_id.to_string(),
                InfluxTimePeriod::new(period),
            )
            .await;
        }
        // Site Stack
        (WasmRequest::SiteStack { period, site_id }, Some(credentials)) => {
            let site_id = urlencoding::decode(site_id).unwrap();
            let _ = send_site_stack_map(
                &cnn,
                tx,
                &credentials.license_key,
                InfluxTimePeriod::new(period),
                site_id.to_string(),
            )
            .await;
        }
        (WasmRequest::RootHeat { period }, Some(credentials)) => {
            let _ = root_heat_map(
                &cnn,
                tx,
                &credentials.license_key,
                InfluxTimePeriod::new(period),
            )
            .await;
        }
        (WasmRequest::SiteHeat { period, site_id }, Some(credentials)) => {
            let _ = site_heat_map(
                &cnn,
                tx,
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
                tx,
                &credentials.license_key,
                InfluxTimePeriod::new(period),
                node_id.to_string(),
                node_name.to_string(),
            )
            .await;
        }
        (WasmRequest::Tree { parent }, Some(credentials)) => {
            send_site_tree(&cnn, tx, &credentials.license_key, parent).await;
        }
        (WasmRequest::SiteInfo { site_id }, Some(credentials)) => {
            send_site_info(&cnn, tx, &credentials.license_key, site_id).await;
        }
        (WasmRequest::SiteParents { site_id }, Some(credentials)) => {
            let site_id = urlencoding::decode(site_id).unwrap();
            send_site_parents(&cnn, tx, &credentials.license_key, &site_id).await;
        }
        (WasmRequest::CircuitParents { circuit_id }, Some(credentials)) => {
            let circuit_id = urlencoding::decode(circuit_id).unwrap();
            send_circuit_parents(&cnn, tx, &credentials.license_key, &circuit_id).await;
        }
        (WasmRequest::RootParents, Some(credentials)) => {
            send_root_parents(&cnn, tx, &credentials.license_key).await;
        }
        (WasmRequest::Search { term }, Some(credentials)) => {
            let _ = omnisearch(&cnn, tx, &credentials.license_key, term).await;
        }
        (WasmRequest::CircuitInfo { circuit_id }, Some(credentials)) => {
            send_circuit_info(&cnn, tx, &credentials.license_key, circuit_id).await;
        }
        (WasmRequest::ExtendedDeviceInfo { circuit_id }, Some(credentials)) => {
            send_extended_device_info(&cnn, tx, &credentials.license_key, circuit_id).await;
        }
        (WasmRequest::SignalNoiseChartExt { period, device_id }, Some(credentials)) => {
            let _ = send_extended_device_snr_graph(
                &cnn,
                tx,
                &credentials.license_key,
                device_id,
                &InfluxTimePeriod::new(period),
            )
            .await;
        }
        (WasmRequest::DeviceCapacityChartExt { period, device_id }, Some(credentials)) => {
            let _ = send_extended_device_capacity_graph(
                &cnn,
                tx,
                &credentials.license_key,
                device_id,
                &InfluxTimePeriod::new(period),
            )
            .await;
        }
        (WasmRequest::ApSignalExt { period, site_name }, Some(credentials)) => {}
        (WasmRequest::ApCapacityExt { period, site_name }, Some(credentials)) => {}
        (_, None) => {
            tracing::error!("No credentials");
        }
        _ => {
            let error = format!("Unknown message: {msg:?}");
            tracing::error!(error);
        }
    }
}

fn serialize_response(response: WasmResponse) -> Vec<u8> {
    let cbor = lts_client::cbor::to_vec(&response).unwrap();
    miniz_oxide::deflate::compress_to_vec(&cbor, 8)
}
