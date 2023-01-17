mod tc_cake;
mod tc_fq_codel;
mod tc_htb;
mod tc_mq;
use anyhow::{Error, Result};
use serde::Serialize;
use serde_json::Value;
use std::process::Command;

#[derive(Debug, Clone, Serialize)]
pub enum QueueType {
    Mq(tc_mq::TcMultiQueue),
    Htb(tc_htb::TcHtb),
    FqCodel(tc_fq_codel::TcFqCodel),
    Cake(tc_cake::TcCake),
    ClsAct,
}

impl QueueType {
    fn parse(kind: &str, map: &serde_json::Map<std::string::String, Value>) -> Result<QueueType> {
        match kind {
            "mq" => Ok(QueueType::Mq(tc_mq::TcMultiQueue::from_json(map)?)),
            "htb" => Ok(QueueType::Htb(tc_htb::TcHtb::from_json(map)?)),
            "fq_codel" => Ok(QueueType::FqCodel(tc_fq_codel::TcFqCodel::from_json(map)?)),
            "cake" => Ok(QueueType::Cake(tc_cake::TcCake::from_json(map)?)),
            "clsact" => Ok(QueueType::ClsAct),
            _ => Err(Error::msg(format!("Unknown queue kind: {kind}"))),
        }
    }
}

/// Separated into a separate function for cleaner benchmark code
pub fn deserialize_tc_tree(json: &str) -> Result<Vec<QueueType>> {
    let mut result = Vec::new();
    let json: Value = serde_json::from_str(json)?;
    if let Value::Array(array) = &json {
        for entry in array.iter() {
            match entry {
                Value::Object(map) => {
                    if let Some(kind) = map.get("kind") {
                        if let Some(kind) = kind.as_str() {
                            let qdisc = QueueType::parse(kind, map)?;
                            result.push(qdisc);
                        }
                    }
                }
                _ => {}
            }
        }
    } else {
        return Err(Error::msg("Unable to parse TC data array"));
    }

    Ok(result)
}

pub(crate) async fn read_tc_queues(interface: &str) -> Result<Vec<QueueType>> {
    let command_output = Command::new("/sbin/tc")
        .args(["-s", "-j", "qdisc", "show", "dev", interface])
        .output()?;
    let json = String::from_utf8(command_output.stdout)?;
    let result = deserialize_tc_tree(&json)?;
    Ok(result)
}
