//! Collects the blackboard functionality together

use lqos_bus::BlackboardSystem;
use serde::Serialize;
use tracing::info;

pub async fn blackboard(subsystem: BlackboardSystem, key: &str, value: &str) {
    let Ok(config) = lqos_config::load_config() else {
        return;
    };
    let req = vec![lqos_bus::BusRequest::BlackboardData {
        subsystem,
        key: key.to_string(),
        value: value.to_string(),
    }];
    let _ = lqos_bus::bus_request(req).await;
}

pub async fn blackboard_blob<T: Serialize>(key: &str, value: T) -> anyhow::Result<()> {
    let config = lqos_config::load_config()?;
    let blob = serde_cbor::to_vec(&value)?;
    let chunks = blob.chunks(1024 * 128);
    info!(
        "Blob {key} is {} bytes long, split into {} chunks",
        blob.len(),
        chunks.len()
    );
    for (i, chunk) in chunks.enumerate() {
        let req = vec![lqos_bus::BusRequest::BlackboardBlob {
            tag: key.to_string(),
            part: i,
            blob: chunk.to_vec(),
        }];
        let err = lqos_bus::bus_request(req).await;
        #[cfg(debug_assertions)]
        if let Err(e) = err {
            tracing::error!("Error writing to blackboard (only an error if lqosd is running): {e:?}");
        }
    }
    Ok(())
}
