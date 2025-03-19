use axum::{Router, response::Html, routing::get};

use crate::db::{
    bandwidth, count_unique_node_ids, count_unique_node_ids_this_week, net_json_nodes,
    shaped_devices,
};

pub async fn stats_viewer() -> anyhow::Result<()> {
    let app = Router::new().route("/", get(index_page));

    log::info!("Listening for web traffic on 0.0.0.0:3000");
    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

async fn index_page() -> Html<String> {
    let result = include_str!("./index.html");
    let unique = count_unique_node_ids().unwrap_or(0);
    let new = count_unique_node_ids_this_week().unwrap_or(0);
    let total_shaped = shaped_devices().unwrap_or(0);
    let net_json_nodes = net_json_nodes().unwrap_or(0);
    let bw = bandwidth().unwrap_or(0) as f64 / 1024.0 / 1024.0;
    let result = result.replace(
        "$$UNIQUE_HOSTS$$",
        &format!("<strong>{unique}</strong> Total LQOS Installs"),
    );
    let result = result.replace(
        "$$NEW_HOSTS$$",
        &format!("<strong>{new}</strong> New Installs This Week"),
    );
    let result = result.replace(
        "$$TOTAL_SHAPED$$",
        &format!("<strong>{total_shaped}</strong> Shaped Devices"),
    );
    let result = result.replace(
        "$$NODES$$",
        &format!("<strong>{net_json_nodes}</strong> Network Hierarchy Nodes"),
    );
    let result = result.replace(
        "$$BW$$",
        &format!("<strong>{bw:.2}</strong> Tbits/s Monitored"),
    );
    Html(result)
}
