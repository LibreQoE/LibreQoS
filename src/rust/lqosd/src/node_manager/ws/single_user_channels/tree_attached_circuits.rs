use crate::node_manager::local_api::tree_attached_circuits::{
    TreeAttachedCircuitsPage, TreeAttachedCircuitsQuery, tree_attached_circuits,
};
use crate::node_manager::ws::messages::{WsResponse, encode_ws_message};
use std::time::Duration;
use tokio::time::MissedTickBehavior;
use tracing::debug;

pub(super) async fn watch_tree_attached_circuits(
    query: TreeAttachedCircuitsQuery,
    tx: tokio::sync::mpsc::Sender<std::sync::Arc<Vec<u8>>>,
) {
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut last_page: Option<TreeAttachedCircuitsPage> = None;

    loop {
        let page = tree_attached_circuits(query.clone());
        let changed = last_page.as_ref() != Some(&page);
        let response = if last_page.is_none() {
            Some(WsResponse::TreeAttachedCircuitsSnapshot { data: page.clone() })
        } else if changed {
            Some(WsResponse::TreeAttachedCircuitsUpdate { data: page.clone() })
        } else {
            None
        };
        last_page = Some(page);

        if let Some(response) = response {
            match encode_ws_message(&response) {
                Ok(payload) => {
                    if tx.send(payload).await.is_err() {
                        debug!("TreeAttachedCircuits watcher channel closed");
                        break;
                    }
                }
                Err(_) => break,
            }
        }

        ticker.tick().await;
    }
}
