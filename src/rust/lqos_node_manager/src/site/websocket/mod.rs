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

use tokio::{
	sync::{
		mpsc::{self, UnboundedSender},
		RwLock
	},
	time::{Duration, Instant},
};
use tokio_stream::wrappers::UnboundedReceiverStream;

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
    let (mut user_tx, mut user_rx) = socket.split();
	let (tx, rx) = mpsc::unbounded_channel();
	let mut rx = UnboundedReceiverStream::new(rx);

	tokio::spawn(rx.forward(user_tx));

	let subscriptions = Mutex::new(HashMap::new());
	let ws_state = Arc::new(WsState { subscriptions });

	let wsstate = ws_state.clone();

	let mut send_task = tokio::spawn(async move {
		let mut count = 0;
		let interval = Duration::from_millis(200);
		let mut next_time = Instant::now() + interval;
		loop {
			// Shaped IPs ~1 updates
			if check_subscription(&wsstate, "shaped_count") && count % 5 == 0 {
				let tx = tx.clone();
				tokio::spawn(async move{
					let message = serde_json::to_string(&WsMessage::new("shaped_count", &json!(tracker::shaped_devices_count().await).to_string(), false)).unwrap();
					tracing::debug!("Sending event: {:#?}", &message);
					tx.send(Ok(Message::Text(message)));
				});
			}

			// Unknown IPs ~1s updates
			if check_subscription(&wsstate, "unknown_count") && count % 5 == 0 {
				let tx = tx.clone();
				tokio::spawn(async move{
					let message = serde_json::to_string(&WsMessage::new("unknown_count", &json!(tracker::unknown_hosts_count().await).to_string(), false)).unwrap();
					tracing::debug!("Sending event: {:#?}", &message);
					tx.send(Ok(Message::Text(message)));
				});
			}
			
			// CPU ~1s updates
			if check_subscription(&wsstate, "cpu") && count % 5 == 0 {
				let tx = tx.clone();
				tokio::spawn(async move{
					let message = serde_json::to_string(&WsMessage::new("cpu", &json!(tracker::cpu_usage().await).to_string(), false)).unwrap();
					tracing::debug!("Sending event: {:#?}", &message);
					tx.send(Ok(Message::Text(message)));
				});
			}

			// RAM ~1s updates
			if check_subscription(&wsstate, "ram") && count % 5 == 0 {
				let tx = tx.clone();
				tokio::spawn(async move{
					let message = serde_json::to_string(&WsMessage::new("ram", &json!(tracker::ram_usage().await).to_string(), false)).unwrap();
					tracing::debug!("Sending event: {:#?}", &message);
					tx.send(Ok(Message::Text(message)));
				});
			}

			// Circuit Throughput ~200ms updates
			if check_subscription(&wsstate, "circuit_throughput") && count % 1 == 0 {
				let tx = tx.clone();
				tokio::spawn(async move{
					let message = serde_json::to_string(&WsMessage::new("circuit_throughput", &json!("").to_string(), false)).unwrap();
					tracing::debug!("Sending event: {:#?}", &message);
					tx.send(Ok(Message::Text(message)));
				});
			}

			// RTT ~5s updates
			if check_subscription(&wsstate, "rtt") && count % 5 == 0 {
				let tx = tx.clone();
				tokio::spawn(async move{
					let message = serde_json::to_string(&WsMessage::new("rtt", &json!(tracker::rtt_histogram().await).to_string(), false)).unwrap();
					tracing::debug!("Sending event: {:#?}", &message);
					tx.send(Ok(Message::Text(message)));
				});
			}

			// Current Throughput ~1s updates
			if check_subscription(&wsstate, "current_throughput") && count % 1 == 0 {
				let tx = tx.clone();
				tokio::spawn(async move{
					let message = serde_json::to_string(&WsMessage::new("current_throughput", &json!(tracker::current_throughput().await).to_string(), false)).unwrap();
					tracing::debug!("Sending event: {:#?}", &message);
					tx.send(Ok(Message::Text(message)));
				});
			}

			// Raw Circuit Queue ~200ms updates
			if check_subscription(&wsstate, "circuit_raw_queue") && count % 5 == 0 {
				let tx = tx.clone();
				tokio::spawn(async move{
					let message = serde_json::to_string(&WsMessage::new("circuit_raw_queue", &json!("").to_string(), false)).unwrap();
					tracing::debug!("Sending event: {:#?}", &message);
					tx.send(Ok(Message::Text(message)));
				});
			}

			// Top 10 Download ~2s updates
			if check_subscription(&wsstate, "top_ten_download") && count % 10 == 0 {
				let tx = tx.clone();
				tokio::spawn(async move{
					let message = serde_json::to_string(&WsMessage::new("top_ten_download", &json!(tracker::top_10_downloaders().await).to_string(), false)).unwrap();
					tracing::debug!("Sending event: {:#?}", &message);
					tx.send(Ok(Message::Text(message)));
				});
			}

			// Worst 10 RTT ~2s updates
			if check_subscription(&wsstate, "worst_ten_rtt") && count % 10 == 0 {
				let tx = tx.clone();
				tokio::spawn(async move{
					let message = serde_json::to_string(&WsMessage::new("worst_ten_rtt", &json!(tracker::worst_10_rtt().await ).to_string(), false)).unwrap();
					tracing::debug!("Sending event: {:#?}", &message);
					tx.send(Ok(Message::Text(message)));
				});
			}

			// Site Funnel ~1s updates
			if check_subscription(&wsstate, "site_funnel") && count % 5 == 0 {
				let tx = tx.clone();
				tokio::spawn(async move{
					let message = serde_json::to_string(&WsMessage::new("site_funnel", &json!(tracker::site_funnel().await).to_string(), false)).unwrap();
					tracing::debug!("Sending event: {:#?}", &message);
					tx.send(Ok(Message::Text(message)));
				});
			}
			
			// Reset counter to keep the number manageable
			count += 1;
			if count == 300 {
				count = 0;
			}
			
			//tokio::time::sleep(std::time::Duration::from_millis(200)).await;
			tokio::time::sleep(next_time - Instant::now()).await;
			next_time += interval;
		}
	});

	let ws_state = ws_state.clone();

    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(Message::Text(text))) = user_rx.next().await {
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