pub(crate) mod circuit;
pub(crate) mod circuit_activity;
pub(crate) mod circuit_count;
pub(crate) mod circuit_live;
pub(crate) mod config;
pub(crate) mod cpu_affinity;
pub(crate) mod dashboard_themes;
pub(crate) mod device_counts;
pub(crate) mod directories;
pub(crate) mod ethernet_caps;
pub(crate) mod executive;
pub(crate) mod executive_cache;
pub(crate) mod flow_explorer;
pub(crate) mod flow_map;
pub mod lts;
pub(crate) mod network_tree;
pub(crate) mod network_tree_lite;
pub(crate) mod node_rate_overrides;
pub(crate) mod node_topology_overrides;
pub(crate) mod packet_analysis;
pub(crate) mod reload_libreqos;
pub(crate) mod scheduler;
pub(crate) mod search;
pub(crate) mod shaped_device_api;
pub(crate) mod shaped_devices_page;
pub(crate) mod throughput_attribution_debug;
pub(crate) mod topology_manager;
pub(crate) mod topology_probes;
pub(crate) mod tree_attached_circuits;
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
        .route(
            "/throughputAttributionDebug",
            get(throughput_attribution_debug::throughput_attribution_debug),
        )
        .layer(Extension(shaper_query))
        .layer(CorsLayer::very_permissive())
        .route_layer(axum::middleware::from_fn(auth_layer))
}
