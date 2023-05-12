pub mod event;
pub mod message;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
		State,
    },
    response::{IntoResponse, Extension, Redirect},
    routing::get,
    Router,
};

use axum::extract::connect_info::ConnectInfo;
use std::{net::SocketAddr, sync::{Arc, Mutex}, collections::HashMap};
use serde_json::json;

use crate::auth::{RequireAuth, AuthContext, Credentials, User, Role};

use crate::lqos;

use message::*;
use event::*;

use tokio::{
	sync::{
        oneshot,
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
	},
};
use crate::AppState;
use crate::site::websocket::event::{Task, WsEvent};

use async_channel::{unbounded};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/ws", get(ws_handler).layer(RequireAuth::login()))
}

#[derive(Debug, Clone)]
pub struct WsState {
	subscriptions: Arc<Mutex<HashMap<String, String>>>,
}

impl WsState {
    fn subscribe(&self, subject: &str, data: &str) {
        self.subscriptions.lock().unwrap().insert(subject.to_string().to_owned(), data.to_string().to_owned());
    }
    fn unsubscribe(&self, subject: &str) {
        self.subscriptions.lock().unwrap().remove(&subject.to_string());
    }
}

pub async fn ws_handler(
	Extension(user): Extension<User>,
	ws: WebSocketUpgrade,
	State(app_state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, app_state))
}

async fn handle_socket(
	mut ws: WebSocket,
	app_state: AppState
) {
    let (mut ws_tx, mut ws_rx) = ws.split();

    let mut async_rx = app_state.async_rx.clone();

	let subscriptions = Arc::new(Mutex::new(HashMap::new()));
	let ws_state = WsState {
        subscriptions: subscriptions
    };

    let wsstate = ws_state.clone();
    let mut send_task = tokio::spawn(async move {
        while let Ok(task_results) = async_rx.recv().await {
            let wsstate = wsstate.clone();
            let event = WsEvent::new(
                "clientupdate",
                WsMessage::new(
                    "",
                    &json!("{}").to_string(),
                    false
                )
            );
            if check_subscription(wsstate, &event.message.subject) {
                let message_text = serde_json::to_string(&event).unwrap();
                ws_tx.send(Message::Text(message_text)).await;
            }
        }
    });

    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(Message::Text(event))) = ws_rx.next().await {
            let wsstate = ws_state.clone();
			WsEvent::decode(&event).process(wsstate);
        }
    });

    tokio::select! {
        _ = (&mut send_task) => recv_task.abort(),
        _ = (&mut recv_task) => send_task.abort(),
    };
}

fn check_subscription(ws_state: WsState, subject: &str) -> bool {
    let mut subscriptions = ws_state.subscriptions.lock().unwrap();
    if subscriptions.contains_key(subject) {
		return true;
    }
	return false;
}