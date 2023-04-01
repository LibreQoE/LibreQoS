use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct WsMessage {
	pub subject: String,
	pub data: String,
	pub packed: bool,
}

impl WsMessage {
    pub fn new(subject: &str, data: &str, packed: bool) -> WsMessage {
        WsMessage {
            subject: subject.to_string(),
            data: data.to_string(),
            packed: false
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WsEvent {
	pub subject: String,
	pub data: WsMessage,
}

impl WsEvent {
    pub fn new(raw_data: &str) -> WsEvent {
        let event: WsEvent = serde_json::from_str(raw_data).unwrap();
        event
    }
}