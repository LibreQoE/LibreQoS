pub(crate) mod tc_cake;
mod tc_fq_codel;
mod tc_htb;
mod tc_mq;
use tracing::warn;
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, Serialize)]
pub enum QueueType {
  Mq(tc_mq::TcMultiQueue),
  Htb(tc_htb::TcHtb),
  FqCodel(tc_fq_codel::TcFqCodel),
  Cake(tc_cake::TcCake),
  ClsAct,
}

impl QueueType {
  fn parse(
    kind: &str,
    map: &serde_json::Map<std::string::String, Value>,
  ) -> Result<QueueType, QDiscError> {
    match kind {
      "mq" => Ok(QueueType::Mq(tc_mq::TcMultiQueue::from_json(map)?)),
      "htb" => Ok(QueueType::Htb(tc_htb::TcHtb::from_json(map)?)),
      "fq_codel" => {
        Ok(QueueType::FqCodel(tc_fq_codel::TcFqCodel::from_json(map)?))
      }
      "cake" => Ok(QueueType::Cake(tc_cake::TcCake::from_json(map)?)),
      "clsact" => Ok(QueueType::ClsAct),
      _ => {
        warn!("I don't know how to parse qdisc type {kind}");
        Err(QDiscError::UnknownQdisc(format!("Unknown queue kind: {kind}")))
      }
    }
  }
}

/// Separated into a separate function for cleaner benchmark code
pub fn deserialize_tc_tree(json: &str) -> Result<Vec<QueueType>, QDiscError> {
  let mut result = Vec::new();
  let json: Value = serde_json::from_str(json)
    .map_err(|_| QDiscError::Json(json.to_string()))?;
  if let Value::Array(array) = &json {
    for entry in array.iter() {
      if let Value::Object(map) = entry {
        if let Some(kind) = map.get("kind") {
          if let Some(kind) = kind.as_str() {
            let qdisc = QueueType::parse(kind, map)?;
            result.push(qdisc);
          }
        }
      }
    }
  } else {
    warn!("Failed to parse TC queue stats data array.");
    return Err(QDiscError::ArrayInvalid);
  }

  Ok(result)
}

#[derive(Error, Debug)]
pub enum QDiscError {
  #[error("Unknown queue kind")]
  UnknownQdisc(String),
  #[error("Error parsing queue information JSON")]
  Json(String),
  #[error("Unable to parse TC data array")]
  ArrayInvalid,
  #[error("Unable to parse Cake Tin options")]
  CakeTin,
  #[error("Unable to parse Cake options")]
  CakeOpts,
  #[error("Unable to parse HTB options")]
  HtbOpts,
  #[error("Unable to parse fq_codel options")]
  CodelOpts,
}

/// Used to extract TC handles without unwrapping.
/// Sets a default value if none can be extracted, rather than
/// bailing on the entire parse run.
#[macro_export]
macro_rules! parse_tc_handle {
  ($target: expr, $value: expr) => {
    let s = $value.as_str();
    if let Some(s) = s {
      if let Ok(handle) = TcHandle::from_string(s) {
        $target = handle;
      } else {
        tracing::info!("Unable to extract TC handle from string");
        $target = TcHandle::default();
      }
    } else {
      tracing::info!("Unable to extract string for TC handle");
      $target = TcHandle::default();
    }
  };
}
