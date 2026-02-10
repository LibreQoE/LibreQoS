//! Websocket handling for the node manager. This module provides a websocket router that can be mounted in
//! the main application. Websocket connections are multi-user, and based on a time-based "ticker". They send
//! out updates to all subscribers at a regular interval, sharing the latest information about the system.
//!
//! Private websocket commands are routed through the same `/ws` entrypoint as pubsub subscriptions.
//! Both types of websocket are authenticated using the auth layer.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use crate::node_manager::auth::{LoginResult, login_from_token};
use crate::node_manager::local_api::{
    circuit, circuit_count, config, cpu_affinity, dashboard_themes, device_counts, flow_explorer,
    flow_map, lts, network_tree, packet_analysis, reload_libreqos, scheduler, search,
    shaped_device_api, unknown_ips, urgent, warnings,
};
use crate::node_manager::shaper_queries_actor::ShaperQueryCommand;
use crate::node_manager::ws::messages::{
    WsHello, WsRequest, WsResponse, encode_ws_message, WS_HANDSHAKE_REQUIREMENT,
};
use crate::node_manager::ws::publish_subscribe::PubSub;
use crate::node_manager::ws::published_channels::PublishedChannels;
use crate::node_manager::ws::ticker::channel_ticker;
use crate::system_stats::SystemStats;
use axum::{
    Extension, Router,
    extract::{
        WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::get,
};
use lqos_bus::BusRequest;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc::Sender;
use serde_cbor::Value as CborValue;
use tracing::{info, warn};

mod publish_subscribe;
mod published_channels;
pub(crate) mod messages;
mod single_user_channels;
mod ticker;

const WS_VERSION: &str = include_str!("../../../../VERSION_STRING");
const HANDSHAKE_TIMEOUT_SECS: u64 = 10;

/// Provides an Axum router for the websocket system. Exposes a single /ws route that supports
/// pubsub subscriptions and private commands.
pub fn websocket_router(
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>,
    system_usage_tx: crossbeam_channel::Sender<tokio::sync::oneshot::Sender<SystemStats>>,
    control_tx: tokio::sync::mpsc::Sender<crate::lts2_sys::control_channel::ControlChannelCommand>,
    shaper_query: Sender<ShaperQueryCommand>,
) -> Router {
    let channels = PubSub::new();
    let ticker_handle = tokio::spawn(channel_ticker(
        channels.clone(),
        bus_tx.clone(),
        system_usage_tx.clone(),
    ));
    tokio::spawn(async move {
        if let Err(err) = ticker_handle.await {
            warn!("Channel ticker task exited: {err}");
        }
    });
    Router::new()
        .route("/ws", get(ws_handler))
        .layer(Extension(channels))
        .layer(Extension(bus_tx.clone()))
        .layer(Extension(control_tx.clone()))
        .layer(Extension(shaper_query.clone()))
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    Extension(channels): Extension<Arc<PubSub>>,
    Extension(bus_tx): Extension<
        Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>,
    >,
    Extension(control_tx): Extension<
        tokio::sync::mpsc::Sender<crate::lts2_sys::control_channel::ControlChannelCommand>,
    >,
    Extension(shaper_query): Extension<Sender<ShaperQueryCommand>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let has_cookie = headers.contains_key(header::COOKIE);
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown");
    info!(
        "WS upgrade requested (cookie present: {has_cookie}, ua: {user_agent})"
    );
    let channels = channels.clone();
    let browser_language = headers
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());
    ws.on_upgrade(move |socket| async move {
        handle_socket(
            socket,
            channels,
            bus_tx,
            control_tx,
            shaper_query,
            browser_language,
        )
        .await;
    })
}

async fn handle_socket(
    socket: WebSocket,
    channels: Arc<PubSub>,
    bus_tx: Sender<(tokio::sync::oneshot::Sender<lqos_bus::BusReply>, BusRequest)>,
    control_tx: tokio::sync::mpsc::Sender<crate::lts2_sys::control_channel::ControlChannelCommand>,
    shaper_query: Sender<ShaperQueryCommand>,
    browser_language: Option<String>,
) {
    info!("Websocket connected");

    let (mut ws_tx, mut ws_rx) = socket.split();

    // Larger buffer helps absorb bursts of pubsub updates without stalling
    // interactive request/response messages.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Arc<Vec<u8>>>(1024);
    let outbound_handle = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if ws_tx.send(Message::Binary((*msg).clone())).await.is_err() {
                break;
            }
        }
    });
    let mut subscribed_channels = HashSet::new();
    let mut handshake_complete = false;
    let mut login = LoginResult::Denied;
    let handshake_timeout = tokio::time::sleep(std::time::Duration::from_secs(
        HANDSHAKE_TIMEOUT_SECS,
    ));
    tokio::pin!(handshake_timeout);

    let hello = WsResponse::Hello {
        hello: WsHello {
            version: WS_VERSION.trim().to_string(),
            requirement: WS_HANDSHAKE_REQUIREMENT.to_string(),
        },
    };
    if send_ws_response(&tx, hello).await {
        outbound_handle.abort();
        let _ = outbound_handle.await;
        return;
    }

    let mut private_state =
        single_user_channels::PrivateState::new(tx.clone(), bus_tx, control_tx, browser_language);
    loop {
        tokio::select! {
            _ = &mut handshake_timeout, if !handshake_complete => {
                warn!("Websocket handshake timed out");
                break;
            }
            inbound = ws_rx.next() => {
                // Received a websocket message
                match inbound {
                    Some(Ok(msg)) => {
                        let should_close = receive_channel_message(
                        msg,
                        channels.clone(),
                        tx.clone(),
                        &mut subscribed_channels,
                        &mut handshake_complete,
                        &mut private_state,
                        &mut login,
                        shaper_query.clone(),
                    ).await;
                        if should_close {
                            break;
                        }
                    }
                    Some(Err(err)) => {
                        warn!("Websocket recv error: {err}");
                        break;
                    }
                    None => break, // The channel has closed
                }
            }
        }
    }
    outbound_handle.abort();
    let _ = outbound_handle.await;
    info!("Websocket disconnected");
}

async fn receive_channel_message(
    msg: Message,
    channels: Arc<PubSub>,
    tx: Sender<Arc<Vec<u8>>>,
    subscribed_channels: &mut HashSet<PublishedChannels>,
    handshake_complete: &mut bool,
    private_state: &mut single_user_channels::PrivateState,
    login: &mut LoginResult,
    shaper_query: Sender<ShaperQueryCommand>,
) -> bool {
    let payload = match msg {
        Message::Binary(data) => data,
        Message::Text(text) => {
            warn!(
                "Websocket text message received (len {}, hint {})",
                text.len(),
                payload_hint(text.as_bytes())
            );
            return true;
        }
        Message::Ping(_) | Message::Pong(_) => {
            return false;
        }
        Message::Close(frame) => {
            warn!("Websocket close frame received: {:?}", frame);
            return true;
        }
    };

    let request = match decode_ws_request(&payload) {
        Ok(request) => request,
        Err(err) => {
            let prefix = payload_prefix_hex(&payload, 24);
            let hint = payload_hint(&payload);
            warn!(
                "Websocket decode failed: {err} (len {}, prefix {prefix}, hint {hint})",
                payload.len()
            );
            return true;
        }
    };

    if !*handshake_complete {
        if let WsRequest::HelloReply(reply) = request {
            if reply.ack != WS_HANDSHAKE_REQUIREMENT {
                warn!("Websocket handshake ack mismatch");
                return true;
            }
            let token = reply.token.trim();
            let login_result = login_from_token(token).await;
            if login_result == LoginResult::Denied {
                warn!("Websocket handshake token rejected");
                return true;
            }
            *login = login_result;
            *handshake_complete = true;
            info!("Websocket handshake completed");
            return false;
        }
        warn!("Websocket message received before handshake complete");
        return true;
    }

    match request {
        WsRequest::Subscribe { channel } => {
            if !subscribed_channels.contains(&channel) {
                channels.subscribe(channel, tx.clone()).await;
                subscribed_channels.insert(channel);
            }
        }
        WsRequest::Unsubscribe { channel } => {
            subscribed_channels.remove(&channel);
            channels.unsubscribe(channel, tx.clone()).await;
        }
        WsRequest::Private(command) => {
            private_state.handle_request(command).await;
        }
        WsRequest::DashletThemes => {
            let response = WsResponse::DashletThemes {
                entries: dashboard_themes::list_theme_entries(),
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::DashletSave { name, entries } => {
            let data = dashboard_themes::DashletSave { name, entries };
            let result = dashboard_themes::save_theme_data(&data);
            let response = match result {
                Ok(_) => WsResponse::DashletSaveResult {
                    ok: true,
                    error: None,
                },
                Err(err) => WsResponse::DashletSaveResult {
                    ok: false,
                    error: Some(err),
                },
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::DashletGet { name } => {
            let entries = dashboard_themes::load_theme_entries(&name);
            let response = WsResponse::DashletTheme { name, entries };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::DashletDelete { name } => {
            let result = dashboard_themes::delete_theme_file(&name);
            let response = match result {
                Ok(_) => WsResponse::DashletDeleteResult {
                    ok: true,
                    error: None,
                },
                Err(err) => WsResponse::DashletDeleteResult {
                    ok: false,
                    error: Some(err),
                },
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::SchedulerStatus => {
            let response = WsResponse::SchedulerStatus {
                data: scheduler::scheduler_status_data(),
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::SchedulerDetails => {
            let response = WsResponse::SchedulerDetails {
                data: scheduler::scheduler_details_data(),
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::DeviceCount => {
            let response = WsResponse::DeviceCount {
                data: device_counts::device_count(),
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::DevicesAll => {
            let response = WsResponse::DevicesAll {
                data: shaped_device_api::all_shaped_devices_data(),
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::NetworkTree => {
            let response = WsResponse::NetworkTree {
                data: network_tree::network_tree_data(),
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::FlowMap => {
            let response = WsResponse::FlowMap {
                data: flow_map::flow_map_data(),
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::CircuitById { id } => {
            let (ok, devices) = match circuit::circuit_by_id_data(&id) {
                Some(devices) => (true, devices),
                None => (false, Vec::new()),
            };
            let response = WsResponse::CircuitByIdResult { id, devices, ok };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::RequestAnalysis { ip } => {
            let response = WsResponse::RequestAnalysisResult {
                data: packet_analysis::request_analysis_data(&ip),
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::CpuAffinitySummary => {
            let response = WsResponse::CpuAffinitySummary {
                data: cpu_affinity::cpu_affinity_summary_data(),
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::CpuAffinityCircuits {
            cpu,
            direction,
            page,
            page_size,
            search,
        } => {
            let query = cpu_affinity::CircuitsQuery {
                direction,
                page,
                page_size,
                search,
            };
            let response = WsResponse::CpuAffinityCircuits {
                data: cpu_affinity::cpu_affinity_circuits_data(cpu, query),
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::CpuAffinityCircuitsAll { direction, search } => {
            let query = cpu_affinity::CircuitsQuery {
                direction,
                page: None,
                page_size: None,
                search,
            };
            let response = WsResponse::CpuAffinityCircuitsAll {
                data: cpu_affinity::cpu_affinity_circuits_all_data(query),
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::CpuAffinityPreviewWeights { direction, search } => {
            let query = cpu_affinity::CircuitsQuery {
                direction,
                page: None,
                page_size: None,
                search,
            };
            let response = WsResponse::CpuAffinityPreviewWeights {
                data: cpu_affinity::cpu_affinity_preview_weights_data(query),
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::CpuAffinitySiteTree => {
            let response = WsResponse::CpuAffinitySiteTree {
                data: cpu_affinity::cpu_affinity_site_tree_data(),
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::AsnList => {
            let response = WsResponse::AsnList {
                data: flow_explorer::asn_list_data(),
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::CountryList => {
            let response = WsResponse::CountryList {
                data: flow_explorer::country_list_data(),
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::ProtocolList => {
            let response = WsResponse::ProtocolList {
                data: flow_explorer::protocol_list_data(),
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::AsnFlowTimeline { asn } => {
            let response = WsResponse::AsnFlowTimeline {
                asn,
                data: flow_explorer::flow_timeline_data(asn),
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::CountryFlowTimeline { iso_code } => {
            let response = WsResponse::CountryFlowTimeline {
                iso_code: iso_code.clone(),
                data: flow_explorer::country_timeline_data(&iso_code),
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::ProtocolFlowTimeline { protocol } => {
            let response = WsResponse::ProtocolFlowTimeline {
                protocol: protocol.clone(),
                data: flow_explorer::protocol_timeline_data(&protocol),
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::GlobalWarnings => {
            let data = warnings::global_warnings_data().await;
            let response = WsResponse::GlobalWarnings { data };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::UrgentStatus => {
            let response = WsResponse::UrgentStatus {
                data: urgent::urgent_status_data(),
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::UrgentList => {
            let response = WsResponse::UrgentList {
                data: urgent::urgent_list_data(),
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::UrgentClear { id } => {
            let ok = urgent::urgent_clear_id(id);
            let response = WsResponse::UrgentClearResult { ok };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::UrgentClearAll => {
            urgent::urgent_clear_all_data();
            let response = WsResponse::UrgentClearAllResult { ok: true };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::UnknownIps => {
            let response = WsResponse::UnknownIps {
                data: unknown_ips::get_unknown_ips(),
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::UnknownIpsClear => {
            let response = WsResponse::UnknownIpsCleared {
                data: unknown_ips::clear_unknown_ips_data(),
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::UnknownIpsCsv => {
            let response = WsResponse::UnknownIpsCsv {
                csv: unknown_ips::unknown_ips_csv_data(),
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::Search { term } => {
            let results = search::search_results(search::SearchRequest { term: term.clone() });
            let response = WsResponse::SearchResults { term, results };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::ReloadLibreQoS => {
            let message = reload_libreqos::reload_libreqos_with_login(*login).await;
            let response = WsResponse::ReloadResult { message };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::LtsTrialConfig => {
            match lts::lts_trial_config_data(*login) {
                Ok(data) => {
                    let response = WsResponse::LtsTrialConfigResult { data };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(StatusCode::FORBIDDEN) => {
                    let response = WsResponse::Error {
                        message: "Unauthorized".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(_) => {
                    let response = WsResponse::Error {
                        message: "Unable to load LTS config".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
            }
        }
        WsRequest::CircuitCount => {
            let response = WsResponse::CircuitCountResult {
                data: circuit_count::circuit_count_data(),
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::LtsSignUp { license_key } => {
            let result = lts::lts_trial_signup_data(license_key).await;
            let (ok, message) = match result {
                Ok(()) => (true, "Ok".to_string()),
                Err(StatusCode::INTERNAL_SERVER_ERROR) => (false, "Invalid license key".to_string()),
                Err(StatusCode::FORBIDDEN) => (false, "Unauthorized".to_string()),
                Err(_) => (false, "Error".to_string()),
            };
            let response = WsResponse::LtsSignUpResult { ok, message };
            if send_ws_response(&tx, response).await {
                return true;
            }
            if ok {
                tokio::spawn(async {
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    std::process::exit(0);
                });
            }
        }
        WsRequest::LtsShaperStatus => {
            match lts::shaper_status_data().await {
                Ok(data) => {
                    let response = WsResponse::LtsShaperStatus { data };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(StatusCode::FORBIDDEN) => {
                    let response = WsResponse::Error {
                        message: "Insight not enabled".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(_) => {
                    let response = WsResponse::Error {
                        message: "Unable to load shaper status".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
            }
        }
        WsRequest::LtsThroughput { seconds } => {
            match lts::throughput_period_data(shaper_query.clone(), seconds).await {
                Ok(data) => {
                    let response = WsResponse::LtsThroughput { seconds, data };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(StatusCode::FORBIDDEN) => {
                    let response = WsResponse::Error {
                        message: "Insight not enabled".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(_) => {
                    let response = WsResponse::Error {
                        message: "Unable to load throughput".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
            }
        }
        WsRequest::LtsPackets { seconds } => {
            match lts::packets_period_data(shaper_query.clone(), seconds).await {
                Ok(data) => {
                    let response = WsResponse::LtsPackets { seconds, data };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(StatusCode::FORBIDDEN) => {
                    let response = WsResponse::Error {
                        message: "Insight not enabled".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(_) => {
                    let response = WsResponse::Error {
                        message: "Unable to load packets".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
            }
        }
        WsRequest::LtsPercentShaped { seconds } => {
            match lts::percent_shaped_period_data(shaper_query.clone(), seconds).await {
                Ok(data) => {
                    let response = WsResponse::LtsPercentShaped { seconds, data };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(StatusCode::FORBIDDEN) => {
                    let response = WsResponse::Error {
                        message: "Insight not enabled".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(_) => {
                    let response = WsResponse::Error {
                        message: "Unable to load percent shaped".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
            }
        }
        WsRequest::LtsFlows { seconds } => {
            match lts::percent_flows_period_data(shaper_query.clone(), seconds).await {
                Ok(data) => {
                    let response = WsResponse::LtsFlows { seconds, data };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(StatusCode::FORBIDDEN) => {
                    let response = WsResponse::Error {
                        message: "Insight not enabled".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(_) => {
                    let response = WsResponse::Error {
                        message: "Unable to load flows".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
            }
        }
        WsRequest::LtsCake { seconds } => {
            match lts::cake_period_data(shaper_query.clone(), seconds).await {
                Ok(data) => {
                    let response = WsResponse::LtsCake { seconds, data };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(StatusCode::FORBIDDEN) => {
                    let response = WsResponse::Error {
                        message: "Insight not enabled".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(_) => {
                    let response = WsResponse::Error {
                        message: "Unable to load cake stats".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
            }
        }
        WsRequest::LtsRttHisto { seconds } => {
            match lts::rtt_histo_period_data(shaper_query.clone(), seconds).await {
                Ok(data) => {
                    let response = WsResponse::LtsRttHisto { seconds, data };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(StatusCode::FORBIDDEN) => {
                    let response = WsResponse::Error {
                        message: "Insight not enabled".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(_) => {
                    let response = WsResponse::Error {
                        message: "Unable to load RTT histogram".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
            }
        }
        WsRequest::LtsTop10Downloaders { seconds } => {
            match lts::top10_downloaders_period_data(shaper_query.clone(), seconds).await {
                Ok(data) => {
                    let response = WsResponse::LtsTop10Downloaders { seconds, data };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(StatusCode::FORBIDDEN) => {
                    let response = WsResponse::Error {
                        message: "Insight not enabled".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(_) => {
                    let response = WsResponse::Error {
                        message: "Unable to load top downloaders".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
            }
        }
        WsRequest::LtsWorst10Rtt { seconds } => {
            match lts::worst10_rtt_period_data(shaper_query.clone(), seconds).await {
                Ok(data) => {
                    let response = WsResponse::LtsWorst10Rtt { seconds, data };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(StatusCode::FORBIDDEN) => {
                    let response = WsResponse::Error {
                        message: "Insight not enabled".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(_) => {
                    let response = WsResponse::Error {
                        message: "Unable to load worst RTT".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
            }
        }
        WsRequest::LtsWorst10Rxmit { seconds } => {
            match lts::worst10_rxmit_period_data(shaper_query.clone(), seconds).await {
                Ok(data) => {
                    let response = WsResponse::LtsWorst10Rxmit { seconds, data };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(StatusCode::FORBIDDEN) => {
                    let response = WsResponse::Error {
                        message: "Insight not enabled".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(_) => {
                    let response = WsResponse::Error {
                        message: "Unable to load worst retransmits".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
            }
        }
        WsRequest::LtsTopFlows { seconds } => {
            match lts::top10_flows_period_data(shaper_query.clone(), seconds).await {
                Ok(data) => {
                    let response = WsResponse::LtsTopFlows { seconds, data };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(StatusCode::FORBIDDEN) => {
                    let response = WsResponse::Error {
                        message: "Insight not enabled".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(_) => {
                    let response = WsResponse::Error {
                        message: "Unable to load top flows".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
            }
        }
        WsRequest::LtsRecentMedian => {
            match lts::recent_medians_data(shaper_query.clone()).await {
                Ok(data) => {
                    let response = WsResponse::LtsRecentMedian { data };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(StatusCode::FORBIDDEN) => {
                    let response = WsResponse::Error {
                        message: "Insight not enabled".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(_) => {
                    let response = WsResponse::Error {
                        message: "Unable to load medians".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
            }
        }
        WsRequest::AdminCheck => {
            let response = WsResponse::AdminCheck {
                ok: config::admin_check_data(*login),
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::GetConfig => {
            match config::get_config_data(*login) {
                Ok(data) => {
                    let response = WsResponse::GetConfig { data };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(StatusCode::FORBIDDEN) => {
                    let response = WsResponse::Error {
                        message: "Unauthorized".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(_) => {
                    let response = WsResponse::Error {
                        message: "Unable to load config".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
            }
        }
        WsRequest::QooProfiles => {
            if *login != crate::node_manager::auth::LoginResult::Admin {
                let response = WsResponse::Error {
                    message: "Unauthorized".to_string(),
                };
                if send_ws_response(&tx, response).await {
                    return true;
                }
            } else {
                match lqos_config::load_qoo_profiles_file() {
                    Ok(file) => {
                        let response = WsResponse::QooProfiles {
                            data: crate::node_manager::ws::messages::QooProfilesSummary {
                                default_profile_id: file.default_profile_id.clone(),
                                profiles: file
                                    .profiles
                                    .iter()
                                    .map(|p| lqos_config::QooProfileInfo {
                                        id: p.id.clone(),
                                        name: p.name.clone(),
                                        description: p.description.clone(),
                                    })
                                    .collect(),
                            },
                        };
                        if send_ws_response(&tx, response).await {
                            return true;
                        }
                    }
                    Err(_) => {
                        let response = WsResponse::Error {
                            message: "Unable to load QoO profiles".to_string(),
                        };
                        if send_ws_response(&tx, response).await {
                            return true;
                        }
                    }
                }
            }
        }
        WsRequest::UpdateConfig { config: cfg } => {
            let result = config::update_lqosd_config_data(*login, cfg).await;
            let (ok, message) = match result {
                Ok(()) => (true, "Ok".to_string()),
                Err(StatusCode::FORBIDDEN) => (false, "Unauthorized".to_string()),
                Err(_) => (false, "Error".to_string()),
            };
            let response = WsResponse::UpdateConfigResult { ok, message };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::UpdateNetworkAndDevices {
            network_json,
            shaped_devices,
        } => {
            let result = config::update_network_and_devices_data(
                *login,
                network_json,
                shaped_devices,
            );
            let (ok, message) = match result {
                Ok(()) => (true, "Ok".to_string()),
                Err(StatusCode::FORBIDDEN) => (false, "Unauthorized".to_string()),
                Err(_) => (false, "Error".to_string()),
            };
            let response = WsResponse::UpdateNetworkAndDevicesResult { ok, message };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::ListNics => {
            match config::list_nics_data(*login) {
                Ok(data) => {
                    let response = WsResponse::ListNics { data };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(StatusCode::FORBIDDEN) => {
                    let response = WsResponse::Error {
                        message: "Unauthorized".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(_) => {
                    let response = WsResponse::Error {
                        message: "Unable to list NICs".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
            }
        }
        WsRequest::NetworkJson => {
            let response = WsResponse::NetworkJson {
                data: config::network_json_data(),
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::AllShapedDevices => {
            let response = WsResponse::AllShapedDevices {
                data: config::all_shaped_devices_data(),
            };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::GetUsers => {
            match config::get_users_data(*login) {
                Ok(data) => {
                    let response = WsResponse::GetUsers { data };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(StatusCode::FORBIDDEN) => {
                    let response = WsResponse::Error {
                        message: "Unauthorized".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
                Err(_) => {
                    let response = WsResponse::Error {
                        message: "Unable to load users".to_string(),
                    };
                    if send_ws_response(&tx, response).await {
                        return true;
                    }
                }
            }
        }
        WsRequest::AddUser {
            username,
            password,
            role,
        } => {
            let result = config::add_user_data(
                *login,
                config::UserRequest {
                    username,
                    password,
                    role,
                },
            );
            let (ok, message) = match result {
                Ok(message) => (true, message),
                Err(StatusCode::FORBIDDEN) => (false, "Unauthorized".to_string()),
                Err(StatusCode::BAD_REQUEST) => (false, "Invalid user data".to_string()),
                Err(_) => (false, "Error".to_string()),
            };
            let response = WsResponse::AddUserResult { ok, message };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::UpdateUser {
            username,
            password,
            role,
        } => {
            let result = config::update_user_data(
                *login,
                config::UserRequest {
                    username,
                    password,
                    role,
                },
            );
            let (ok, message) = match result {
                Ok(message) => (true, message),
                Err(StatusCode::FORBIDDEN) => (false, "Unauthorized".to_string()),
                Err(StatusCode::BAD_REQUEST) => (false, "Invalid user data".to_string()),
                Err(_) => (false, "Error".to_string()),
            };
            let response = WsResponse::UpdateUserResult { ok, message };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::DeleteUser { username } => {
            let result = config::delete_user_data(*login, username);
            let (ok, message) = match result {
                Ok(message) => (true, message),
                Err(StatusCode::FORBIDDEN) => (false, "Unauthorized".to_string()),
                Err(StatusCode::BAD_REQUEST) => (false, "Invalid user data".to_string()),
                Err(_) => (false, "Error".to_string()),
            };
            let response = WsResponse::DeleteUserResult { ok, message };
            if send_ws_response(&tx, response).await {
                return true;
            }
        }
        WsRequest::HelloReply(_) => {}
    }
    false
}

async fn send_ws_response(tx: &Sender<Arc<Vec<u8>>>, response: WsResponse) -> bool {
    let payload = match encode_ws_message(&response) {
        Ok(payload) => payload,
        Err(_) => return true,
    };
    match tx.try_send(payload) {
        Ok(()) => false,
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
            warn!("Websocket outbound queue full; closing connection");
            true
        }
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => true,
    }
}

fn decode_ws_request(payload: &[u8]) -> Result<WsRequest, String> {
    let prefix = payload_prefix_hex(payload, 24);
    let hint = payload_hint(payload);
    match serde_cbor::from_slice::<WsRequest>(payload) {
        Ok(request) => Ok(request),
        Err(err) => {
            if let Ok(CborValue::Map(map)) = serde_cbor::from_slice::<CborValue>(payload) {
                if map.len() == 1 {
                    let (key, value) = map.into_iter().next().unwrap();
                    if matches!(value, CborValue::Map(ref inner) if inner.is_empty()) {
                        let mut normalized_map = BTreeMap::new();
                        normalized_map.insert(key, CborValue::Null);
                        let normalized = CborValue::Map(normalized_map);
                        if let Ok(request) =
                            serde_cbor::value::from_value::<WsRequest>(normalized)
                        {
                            return Ok(request);
                        }
                    }
                }
            }
            Err(format!(
                "{err} (len {}, prefix {prefix}, hint {hint})",
                payload.len()
            ))
        }
    }
}

fn payload_prefix_hex(payload: &[u8], max_len: usize) -> String {
    let mut out = String::new();
    let len = payload.len().min(max_len);
    for (idx, byte) in payload.iter().take(len).enumerate() {
        if idx > 0 {
            out.push(' ');
        }
        out.push_str(&format!("{:02x}", byte));
    }
    out
}

fn payload_hint(payload: &[u8]) -> &'static str {
    let mut idx = 0;
    while idx < payload.len() && payload[idx].is_ascii_whitespace() {
        idx += 1;
    }
    if idx >= payload.len() {
        return "empty";
    }
    match payload[idx] {
        b'{' | b'[' => "looks like JSON",
        b'"' => "looks like quoted text",
        _ => "binary",
    }
}
