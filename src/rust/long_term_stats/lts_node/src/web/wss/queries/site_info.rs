use super::site_tree::tree_to_host;
use pgdb::sqlx::{Pool, Postgres};
use serde::Serialize;
use tokio::sync::mpsc::Sender;
use wasm_pipe_types::{SiteTree, WasmResponse, SiteOversubscription};

#[derive(Serialize)]
struct SiteInfoMessage {
    msg: String,
    data: SiteTree,
}

pub async fn send_site_info(
    cnn: &Pool<Postgres>,
    tx: Sender<WasmResponse>,
    key: &str,
    site_id: &str,
) {
    let (host, oversub) = tokio::join!(
        pgdb::get_site_info(cnn, key, site_id),
        pgdb::get_oversubscription(cnn, key, site_id)
    );

    if let Ok(host) = host {
        if let Ok(oversub) = oversub {
            let host = tree_to_host(host);
            let oversubscription = SiteOversubscription {
                dlmax: oversub.dlmax,
                dlmin: oversub.dlmin,
                devicecount: oversub.devicecount,
            };
            tx.send(WasmResponse::SiteInfo { data: host, oversubscription }).await.unwrap();
        } else {
            tracing::error!("{oversub:?}");
        }
    } else {
        tracing::error!("{host:?}");
    }
}
