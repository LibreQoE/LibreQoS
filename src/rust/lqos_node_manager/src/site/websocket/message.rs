use serde::{Serialize, Deserialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WsMessage {
	pub subject: String,
	pub content: String,
	pub packed: bool,
}

impl WsMessage {
    pub fn new(subject: &str, content: &str, packed: bool) -> WsMessage {
        WsMessage {
            subject: subject.to_string(),
            content: content.to_string(),
            packed: packed
        }
    }
}