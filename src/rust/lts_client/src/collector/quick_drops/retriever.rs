//! Async reader/parser for tc -s -j qdisc show dev (whatever)
use thiserror::Error;
use tokio::process::Command;
pub use crate::collector::CakeStats;
use super::queue_structure::{read_queueing_structure, QueueNode};

#[derive(Debug, Error)]
pub(crate) enum AsyncQueueReaderMessage {
    #[error("Unable to figure out the current queue structure")]
    QueueStructure,
    #[error("Unable to query the interface with tc")]
    FetchRawFail,
    #[error("Unable to fetch stdout")]
    FetchStdout,
    #[error("JSON decode error")]
    JsonDecode,
}

pub(crate) struct AsyncQueueReader {
    pub(crate) interface: String,
}

impl AsyncQueueReader {
    pub(crate) fn new<S: ToString>(interface: S) -> Self {
        Self {
            interface: interface.to_string(),
        }
    }

    pub(crate) async fn run(&self) -> Result<Option<Vec<CakeStats>>, AsyncQueueReaderMessage> {
        let mut result = None;
        if let Ok(queue_map) =
            read_queueing_structure().map_err(|_| AsyncQueueReaderMessage::QueueStructure)
        {
            if let Ok(raw) = self.fetch_raw().await {
                let stats = self.quick_parse(&raw, &queue_map).await?;
                result = Some(stats);
            } else {
                log::error!("Unable to fetch raw tc output");
            }
        }

        Ok(result)
    }

    pub(crate) async fn run_on_a_stick(&self) -> Result<(Option<Vec<CakeStats>>, Option<Vec<CakeStats>>), AsyncQueueReaderMessage> {
        let mut result = (None, None);
        if let Ok(queue_map) =
            read_queueing_structure().map_err(|_| AsyncQueueReaderMessage::QueueStructure)
        {
            if let Ok(raw) = self.fetch_raw().await {
                let stats = self.quick_parse_stick(&raw, &queue_map).await?;
                result = (Some(stats.0), Some(stats.1));
            } else {
                log::error!("Unable to fetch raw tc output");
            }
        }

        Ok(result)
    }

    async fn fetch_raw(&self) -> Result<String, AsyncQueueReaderMessage> {
        let command_output = Command::new("/sbin/tc")
            .args(["-s", "-j", "qdisc", "show", "dev", &self.interface])
            .output()
            .await
            .map_err(|_| AsyncQueueReaderMessage::FetchRawFail)?;
        let json = String::from_utf8(command_output.stdout)
            .map_err(|_| AsyncQueueReaderMessage::FetchStdout)?;
        Ok(json)
    }

    async fn quick_parse(&self, raw: &str, structure: &[QueueNode]) -> Result<Vec<CakeStats>, AsyncQueueReaderMessage> {
        let mut result = Vec::with_capacity(structure.len());

        let json = serde_json::from_str::<serde_json::Value>(raw)
            .map_err(|_| AsyncQueueReaderMessage::JsonDecode)?;

        if let Some(array) = json.as_array() {
            for entry in array.iter() {
                if let Some(map) = entry.as_object() {
                    if let (Some(kind), Some(handle)) =
                        (map.get_key_value("kind"), map.get_key_value("parent"))
                    {
                        if let (Some("cake"), Some(handle)) = (kind.1.as_str(), handle.1.as_str()) {
                            structure.iter().for_each(|node| {
                                if node.class_id.to_string() == handle.to_string() {
                                    if let Some(circuit_id) = &node.circuit_id {
                                        let mut stats = CakeStats {
                                            circuit_id: circuit_id.to_string(),
                                            ..Default::default()
                                        };
                                        if let Some(serde_json::Value::Number(drops)) = map.get("drops") {
                                            stats.drops = drops.as_u64().unwrap_or(0);
                                        }
                                        if let Some(serde_json::Value::Number(marks)) = map.get("ecn_mark") {
                                            stats.marks = marks.as_u64().unwrap_or(0);
                                        }
                                        result.push(stats);
                                    }
                                }
                            })
                        }
                    }
                }

                // Be good async citizens and don't eat the CPU
                tokio::task::yield_now().await;
            }
        }

        Ok(result)
    }

    async fn quick_parse_stick(&self, raw: &str, structure: &[QueueNode]) -> Result<(Vec<CakeStats>, Vec<CakeStats>), AsyncQueueReaderMessage> {
        let mut down = Vec::with_capacity(structure.len());
        let mut up = Vec::with_capacity(structure.len());

        let json = serde_json::from_str::<serde_json::Value>(raw)
            .map_err(|_| AsyncQueueReaderMessage::JsonDecode)?;

        if let Some(array) = json.as_array() {
            for entry in array.iter() {
                if let Some(map) = entry.as_object() {
                    if let (Some(kind), Some(handle)) =
                        (map.get_key_value("kind"), map.get_key_value("parent"))
                    {
                        if let (Some("cake"), Some(handle)) = (kind.1.as_str(), handle.1.as_str()) {
                            structure.iter().for_each(|node| {
                                if node.class_id.to_string() == handle.to_string() {
                                    if let Some(circuit_id) = &node.circuit_id {
                                        let mut stats = CakeStats {
                                            circuit_id: circuit_id.to_string(),
                                            ..Default::default()
                                        };
                                        if let Some(serde_json::Value::Number(drops)) = map.get("drops") {
                                            stats.drops = drops.as_u64().unwrap_or(0);
                                        }
                                        if let Some(serde_json::Value::Number(marks)) = map.get("ecn_mark") {
                                            stats.marks = marks.as_u64().unwrap_or(0);
                                        }
                                        down.push(stats);
                                    }
                                } else if node.up_class_id.to_string() == handle.to_string() {
                                    if let Some(circuit_id) = &node.circuit_id {
                                        let mut stats = CakeStats {
                                            circuit_id: circuit_id.to_string(),
                                            ..Default::default()
                                        };
                                        if let Some(serde_json::Value::Number(drops)) = map.get("drops") {
                                            stats.drops = drops.as_u64().unwrap_or(0);
                                        }
                                        if let Some(serde_json::Value::Number(marks)) = map.get("ecn_mark") {
                                            stats.marks = marks.as_u64().unwrap_or(0);
                                        }
                                        up.push(stats);
                                    }
                                }
                            })
                        }
                    }
                }

                // Be good async citizens and don't eat the CPU
                tokio::task::yield_now().await;
            }
        }

        Ok((down, up))
    }
}
