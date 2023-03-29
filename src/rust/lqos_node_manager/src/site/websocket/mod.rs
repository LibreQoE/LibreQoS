use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
		ConnectInfo,
		State,
        TypedHeader,
    },
    response::IntoResponse,
    routing::get,
	Extension,
	Json,
    Router,
};

use std::{ops::ControlFlow, sync::{Arc, Mutex}, collections::HashSet};
use serde_json::{Result, Value, json};
use serde::{Serialize, Deserialize};
use std::net::SocketAddr;

use crate::lqos::tracker;

use futures::{
	sink::SinkExt,
	stream::{
		StreamExt, SplitSink, SplitStream
	}
};
use crate::auth;
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/ws", get(ws_handler))
}

#[derive(Debug, Serialize, Deserialize)]
struct EventMessage {
	subject: String,
	data: serde_json::Value,
	packed: bool,
}

#[derive(Debug)]
struct WsState {
	subscriptions: Mutex<HashSet<String>>,
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

	let subscriptions = Mutex::new(HashSet::new());
	let ws_state = Arc::new(WsState { subscriptions });
	
	let wsstate = ws_state.clone();

	let mut send_task = tokio::spawn(async move {
		let mut count = 0;
		loop {
			if check_subscription(&wsstate, "update_shaped_count") && count % 25 == 0 {
				let data = json!(tracker::shaped_devices_count());
				let message_contents = EventMessage {
					subject: "update_shaped_count".to_string(),
					data: data,
					packed: false
				};
				tx.send(Message::Text(serde_json::to_string(&message_contents).unwrap())).await;
			}

			// Unknown IPs ~5 second updates
			if check_subscription(&wsstate, "update_unknown_count") && count % 25 == 0 {
				let data = json!(tracker::unknown_hosts_count());
				let message_contents = EventMessage {
					subject: "update_unknown_count".to_string(),
					data: data,
					packed: false
				};
				tx.send(Message::Text(serde_json::to_string(&message_contents).unwrap())).await;
			}
			
			// CPU ~2 second updates
			if check_subscription(&wsstate, "update_cpu") && count % 10 == 0 {
				let data = json!(tracker::cpu_usage());
				let message_contents = EventMessage {
					subject: "update_cpu".to_string(),
					data: data,
					packed: false
				};
				tx.send(Message::Text(serde_json::to_string(&message_contents).unwrap())).await;
			}

			// RAM ~30 second updates
			if check_subscription(&wsstate, "update_ram") && count % 150 == 0 {
				let data = json!(tracker::ram_usage());
				let message_contents = EventMessage {
					subject: "update_ram".to_string(),
					data: data,
					packed: false
				};
				tx.send(Message::Text(serde_json::to_string(&message_contents).unwrap())).await;
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
			tracing::debug!("Received message {:#?}", text);
			let event_message: EventMessage = serde_json::from_str(&text).unwrap();
			if event_message.subject == "subscribe" {
				let subscribe_message: EventMessage = serde_json::from_str(&event_message.data.to_string()).unwrap();
				tracing::debug!("Received subscribe request {:#?}", subscribe_message.subject);
				&ws_state.subscriptions.lock().unwrap().insert(subscribe_message.subject.to_string().to_owned());
			} else if event_message.subject == "unsubscribe" {
				let unsubscribe_message: EventMessage = serde_json::from_str(&event_message.data.to_string()).unwrap();
				tracing::debug!("Received unsubscribe request {:#?}", unsubscribe_message.subject);
				&ws_state.subscriptions.lock().unwrap().remove(&unsubscribe_message.subject.to_string());
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
    if subscriptions.contains(subject) {
		return true;
    }
	return false;
}