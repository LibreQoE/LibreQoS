use axum::extract::ws::WebSocket;
use pgdb::{
    sqlx::{Pool, Postgres},
    TreeNode,
};
use wasm_pipe_types::SiteTree;
use crate::web::wss::send_response;

pub async fn send_site_tree(cnn: &Pool<Postgres>, socket: &mut WebSocket, key: &str, parent: &str) {
    let tree = pgdb::get_site_tree(cnn, key, parent).await.unwrap();
    let tree = tree
        .into_iter()
        .map(tree_to_host)
        .collect::<Vec<SiteTree>>();

    send_response(socket, wasm_pipe_types::WasmResponse::SiteTree { data: tree }).await;
}

pub(crate) fn tree_to_host(row: TreeNode) -> SiteTree {
    SiteTree {
        index: row.index,
        site_name: row.site_name,
        site_type: row.site_type,
        parent: row.parent,
        max_down: row.max_down,
        max_up: row.max_up,
        current_down: row.current_down,
        current_up: row.current_up,
        current_rtt: row.current_rtt,
    }
}
