pub(crate) mod circuit;
pub(crate) mod circuit_count;
pub(crate) mod config;
pub(crate) mod cpu_affinity;
pub(crate) mod dashboard_themes;
pub(crate) mod device_counts;
pub(crate) mod flow_explorer;
pub(crate) mod flow_map;
pub mod lts;
pub(crate) mod network_tree;
pub(crate) mod packet_analysis;
pub(crate) mod reload_libreqos;
pub(crate) mod scheduler;
pub(crate) mod search;
pub(crate) mod shaped_device_api;
pub(crate) mod unknown_ips;
pub(crate) mod urgent;
pub(crate) mod warnings;

use crate::node_manager::auth::auth_layer;
use crate::node_manager::shaper_queries_actor::ShaperQueryCommand;
use axum::routing::get;
use axum::{Extension, Router};
use tower_http::cors::CorsLayer;

pub fn local_api(shaper_query: tokio::sync::mpsc::Sender<ShaperQueryCommand>) -> Router {
    Router::new()
        .route("/pcapDump/:id", get(packet_analysis::pcap_dump))
        .layer(Extension(shaper_query))
        .layer(CorsLayer::very_permissive())
        .route_layer(axum::middleware::from_fn(auth_layer))
}
