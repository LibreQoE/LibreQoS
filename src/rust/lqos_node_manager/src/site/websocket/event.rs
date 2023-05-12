use crate::AppState;
use serde::{Serialize, Deserialize};

use std::net::IpAddr;
use crate::WsMessage;
use crate::site::websocket::WsState;
use std::sync::Arc;

#[derive(Debug, Serialize, Deserialize)]
pub enum Task {
    #[serde(rename = "clientupdate")]
    ClientUpdate,
    #[serde(rename = "dnslookup")]
    DNSLookup,
    #[serde(rename = "subscribe")]
    Subscribe,
    #[serde(rename = "unsubscribe")]
    Unsubscribe,
    Unknown
}

impl From<&str> for Task {
    fn from(s: &str) -> Self {
        match s {
            "clientupdate" => Task::ClientUpdate,
            "dnslookup" => Task::DNSLookup,
            "subscribe" => Task::Subscribe,
            "unsubscribe" => Task::Unsubscribe,
            _ => Task::Unknown
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WsEvent {
	pub task: Task,
	pub message: WsMessage,
    pub instructor: Option<String>,
}

impl WsEvent {
    pub fn new(task: &str, message: WsMessage) -> Self {
        WsEvent {
            task: task.into(),
            message: message,
            instructor: None
        }
    }

    pub fn decode(raw: &str) -> Self {
        let event: WsEvent = serde_json::from_str(raw).unwrap();
        event
    }

    pub fn process(&self, ws_state: WsState) {
        match self.task {
            Task::Subscribe => {
                ws_state.subscribe(&self.message.subject, &self.message.content);
            },
            Task::Unsubscribe => {
                ws_state.unsubscribe(&self.message.subject);
            },
            Task::DNSLookup => {
                if let Ok(ip) = &self.message.content.parse::<IpAddr>() {
                    //let _ = user_tx.send(Message::Text(lqos::tracker::lookup_dns(ip)));
                }
            },
            _ => {
                tracing::debug!("Received unknown event {:#?}", self);
            },
        }
    }
}