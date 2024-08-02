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
mod circuit;
mod packet_analysis;
mod flow_map;
mod warnings;
mod flow_explorer;

use axum::{Extension, Router};
use axum::routing::{get, post};
use crate::node_manager::auth::auth_layer;
use tower_http::cors::CorsLayer;
use crate::lts2::ControlSender;

pub fn local_api(lts2_control_channel: ControlSender) -> Router {
    Router::new()
        .route("/dashletThemes", get(dashboard_themes::list_themes))
        .route("/dashletSave", post(dashboard_themes::save_theme))
        .route("/dashletDelete", post(dashboard_themes::delete_theme))
        .route("/dashletGet", post(dashboard_themes::get_theme))
        .route("/versionCheck", get(version_check::version_check))
        .route("/deviceCount", get(device_counts::count_users))
        .route("/devicesAll", get(shaped_device_api::all_shaped_devices))
        .route("/networkTree", get(network_tree::get_network_tree))
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
        .route("/updateConfig", post(config::update_lqosd_config))
        .route("/updateNetworkAndDevices", post(config::update_network_and_devices))
        .route("/circuitById", post(circuit::get_circuit_by_id))
        .route("/requestAnalysis/:ip", get(packet_analysis::request_analysis))
        .route("/pcapDump/:id", get(packet_analysis::pcap_dump))
        .route("/flowMap", get(flow_map::flow_lat_lon))
        .route("/globalWarnings", get(warnings::get_global_warnings))
        .route("/asnList", get(flow_explorer::asn_list))
        .route("/countryList", get(flow_explorer::country_list))
        .route("/protocolList", get(flow_explorer::protocol_list))
        .route("/flowTimeline/:asn_id", get(flow_explorer::flow_timeline))
        .route("/countryTimeline/:iso_code", get(flow_explorer::country_timeline))
        .route("/protocolTimeline/:protocol", get(flow_explorer::protocol_timeline))
        .route("/ltsSignUp", post(lts::lts_trial_signup))
        .route("/ltsShaperStatus", get(lts::shaper_status_from_lts))
        .layer(Extension(lts2_control_channel)) // Add the LTS control channel as an extension
        .layer(CorsLayer::very_permissive())
        .route_layer(axum::middleware::from_fn(auth_layer))
}