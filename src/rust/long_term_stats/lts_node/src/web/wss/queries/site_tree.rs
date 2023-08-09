use pgdb::{
    sqlx::{Pool, Postgres},
    TreeNode,
};
use tokio::sync::mpsc::Sender;
use wasm_pipe_types::{SiteTree, WasmResponse};

#[tracing::instrument(skip(cnn, tx, key, parent))]
pub async fn send_site_tree(cnn: &Pool<Postgres>, tx: Sender<WasmResponse>, key: &str, parent: &str) {
    let tree = pgdb::get_site_tree(cnn, key, parent).await.unwrap();
    let tree = tree
        .into_iter()
        .map(tree_to_host)
        .collect::<Vec<SiteTree>>();

    tx.send(WasmResponse::SiteTree { data: tree }).await.unwrap();
}

pub(crate) fn tree_to_host(row: TreeNode) -> SiteTree {
    SiteTree {
        index: row.index,
        site_name: row.site_name,
        site_type: row.site_type,
        parent: row.parent,
        max_down: row.max_down,
        max_up: row.max_up,
        current_down: row.current_down * 8,
        current_up: row.current_up * 8,
        current_rtt: row.current_rtt,
    }
}
