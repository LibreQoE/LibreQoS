use pgdb::sqlx::{Pool, Postgres};
use tokio::sync::mpsc::Sender;
use wasm_pipe_types::WasmResponse;

#[tracing::instrument(skip(cnn, tx, key, site_name))]
pub async fn send_site_parents(
    cnn: &Pool<Postgres>,
    tx: Sender<WasmResponse>,
    key: &str,
    site_name: &str,
) {
    if let Ok(parents) = pgdb::get_parent_list(cnn, key, site_name).await {
        tx.send(WasmResponse::SiteParents { data: parents }).await.unwrap();
    }

    let child_result = pgdb::get_child_list(cnn, key, site_name).await;
    if let Ok(children) = child_result {
        tx.send(WasmResponse::SiteChildren { data: children }).await.unwrap();
    } else {
        tracing::error!("Error getting children: {:?}", child_result);
    }
}

pub async fn send_circuit_parents(
    cnn: &Pool<Postgres>,
    tx: Sender<WasmResponse>,
    key: &str,
    circuit_id: &str,
) {
    if let Ok(parents) = pgdb::get_circuit_parent_list(cnn, key, circuit_id).await {
        tx.send(WasmResponse::SiteParents { data: parents }).await.unwrap();
    }
}

pub async fn send_root_parents(
    cnn: &Pool<Postgres>,
    tx: Sender<WasmResponse>,
    key: &str,
) {
    let site_name = "Root";
    let child_result = pgdb::get_child_list(cnn, key, site_name).await;
    if let Ok(children) = child_result {
        tx.send(WasmResponse::SiteChildren { data: children }).await.unwrap();
    } else {
        tracing::error!("Error getting children: {:?}", child_result);
    }
}