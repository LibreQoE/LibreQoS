mod dashboard_themes;
mod version_check;
mod device_counts;
mod shaped_device_api;
mod network_tree;
mod support;
mod lts;
mod search;
mod unknown_ips;
mod reload_libreqos;
mod config;

use axum::Router;
use axum::routing::{get, post};
use crate::node_manager::auth::auth_layer;

pub fn local_api() -> Router {
    Router::new()
        .route("/dashletThemes", get(dashboard_themes::list_themes))
        .route("/dashletSave", post(dashboard_themes::save_theme))
        .route("/dashletDelete", post(dashboard_themes::delete_theme))
        .route("/dashletGet", post(dashboard_themes::get_theme))
        .route("/versionCheck", get(version_check::version_check))
        .route("/deviceCount", get(device_counts::count_users))
        .route("/devicesAll", get(shaped_device_api::all_shaped_devices))
        .route("/networkTree/:parent", get(network_tree::get_network_tree))
        .route("/sanity", get(support::run_sanity_check))
        .route("/gatherSupport", post(support::gather_support_data))
        .route("/submitSupport", post(support::submit_support_data))
        .route("/ltsCheck", get(lts::stats_check))
        .route("/search", post(search::search))
        .route("/unknownIps", get(unknown_ips::unknown_ips))
        .route("/unknownIpsCsv", get(unknown_ips::unknown_ips_csv))
        .route("/reloadLqos", get(reload_libreqos::reload_libreqos))
        .route("/adminCheck", get(config::admin_check))
        .route("/getConfig", get(config::get_config))
        .route("/listNics", get(config::list_nics))
        .route("/networkJson", get(config::network_json))
        .route("/allShapedDevices", get(config::all_shaped_devices))
        .route_layer(axum::middleware::from_fn(auth_layer))
}