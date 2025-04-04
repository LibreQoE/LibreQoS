use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub enum LinkMapping {
    Ethernet,
    DevicePair(String, String),
}