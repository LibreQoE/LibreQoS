pub mod event;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
		State,
    },
    response::IntoResponse,
    routing::get,
    Router,
};

use std::{sync::{Arc, Mutex}, collections::HashMap};
use serde_json::json;
use crate::auth::{self, RequireAuth};

use crate::lqos::tracker;

use crate::site::websocket::event::{WsEvent, WsMessage};

use futures::{
	sink::SinkExt,
	stream::{
		StreamExt
	}
};
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/ws", get(ws_handler).layer(RequireAuth::login()))
}

#[derive(Debug)]
struct WsState {
	subscriptions: Mutex<HashMap<String, String>>,
}

pub async fn ws_handler(
	ws: WebSocketUpgrade,
	State(state): State<AppState>,
) -> impl IntoResponse {
	ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(
	socket: WebSocket,
	state: AppState,
) {
    let (mut tx, mut rx) = socket.split();

	let subscriptions = Mutex::new(HashMap::new());
	let ws_state = Arc::new(WsState { subscriptions });
	
	let wsstate = ws_state.clone();

	let mut send_task = tokio::spawn(async move {
		let mut count = 0;
		loop {
			// Shaped IPs ~5 second updates
			if check_subscription(&wsstate, "shaped_count") && count % 25 == 0 {
				let message = serde_json::to_string(&WsMessage::new("shaped_count", &json!(tracker::shaped_devices_count().await).to_string(), false)).unwrap();
				tracing::debug!("Sending event: {:#?}", &message);
				tx.send(Message::Text(message)).await;
			}

			// Unknown IPs ~5 second updates
			if check_subscription(&wsstate, "unknown_count") && count % 25 == 0 {
				let message = serde_json::to_string(&WsMessage::new("unknown_count", &json!(tracker::unknown_hosts_count().await).to_string(), false)).unwrap();
				tracing::debug!("Sending event: {:#?}", &message);
				tx.send(Message::Text(message)).await;
			}
			
			// CPU ~2 second updates
			if check_subscription(&wsstate, "cpu") && count % 10 == 0 {
				let message = serde_json::to_string(&WsMessage::new("cpu", &json!(tracker::cpu_usage().await).to_string(), false)).unwrap();
				tracing::debug!("Sending event: {:#?}", &message);
				tx.send(Message::Text(message)).await;
			}

			// RAM ~30 second updates
			if check_subscription(&wsstate, "ram") && count % 150 == 0 {
				let message = serde_json::to_string(&WsMessage::new("ram", &json!(tracker::ram_usage().await).to_string(), false)).unwrap();
				tracing::debug!("Sending event: {:#?}", &message);
				tx.send(Message::Text(message)).await;
			}

			if check_subscription(&wsstate, "circuit_throughput") && count % 150 == 0 {
				let message = serde_json::to_string(&WsMessage::new("circuit_throughput", &json!(tracker::ram_usage().await).to_string(), false)).unwrap();
				tracing::debug!("Sending event: {:#?}", &message);
				tx.send(Message::Text(message)).await;
			}

			if check_subscription(&wsstate, "circuit_raw_queue") && count % 150 == 0 {
				
			}
			
			// Reset counter to keep the number manageable
			count += 1;
			if count == 300 {
				count = 0;
			}

			tokio::time::sleep(std::time::Duration::from_millis(200)).await;
		}
	});

	let ws_state = ws_state.clone();

    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(Message::Text(text))) = rx.next().await {
			let event = WsEvent::new(&text);
			match event.subject.as_str() {
				"subscribe" => {
					let message: WsMessage = event.data;
					tracing::debug!("Received subscribe event {:#?}", &message.subject);
					&ws_state.subscriptions.lock().unwrap().insert(message.subject.to_string().to_owned(), message.data.to_string().to_owned());
				},
				"unsubscribe" => {
					let message: WsMessage = event.data;
					tracing::debug!("Received unsubscribe event {:#?}", &message.subject);
					&ws_state.subscriptions.lock().unwrap().remove(&message.subject.to_string());
				},
				_ => {
					tracing::debug!("Received unknown event {:#?}", event);
				}
			}
        }
    });

    tokio::select! {
        _ = (&mut send_task) => recv_task.abort(),
        _ = (&mut recv_task) => send_task.abort(),
    };
}

fn check_subscription(state: &WsState, subject: &str) -> bool {
    let mut subscriptions = state.subscriptions.lock().unwrap();
    if subscriptions.contains_key(subject) {
		return true;
    }
	return false;
}