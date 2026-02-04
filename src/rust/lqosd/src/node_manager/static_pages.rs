use crate::node_manager::auth::auth_layer;
use crate::node_manager::template::apply_templates;
use anyhow::{Result, bail};
use axum::Router;
use lqos_config::load_config;
use std::path::Path;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};

pub(super) fn vendor_route() -> Result<Router> {
    let config = load_config()?;
    let vendor_path = Path::new(&config.lqos_directory)
        .join("bin")
        .join("static2")
        .join("vendor");

    if !vendor_path.exists() {
        bail!("Vendor path not found for webserver (bin/static2/vendor/");
    }

    let router = Router::new().nest_service("/", ServeDir::new(vendor_path));

    Ok(router)
}

pub(super) fn static_routes() -> Result<Router> {
    let config = load_config()?;

    // Add HTML pages to serve directly to this list, otherwise
    // they won't have template + authentication applied to them.
    let html_pages = [
        "index.html",
        "shaped_devices.html",
        "tree.html",
        "help.html",
        "unknown_ips.html",
        "configuration.html",
        "circuit.html",
        "flow_map.html",
        "all_tree_sankey.html",
        "asn_explorer.html",
        "api_docs.html",
        "chatbot.html",
        "lts_trial.html",
        "lts_trial_success.html",
        "lts_trial_fail.html",
        "executive_worst_sites.html",
        "executive_oversubscribed_sites.html",
        "executive_sites_due_upgrade.html",
        "executive_circuits_due_upgrade.html",
        "executive_top_asns.html",
        "executive_heatmap_rtt.html",
        "executive_heatmap_retransmit.html",
        "executive_heatmap_download.html",
        "executive_heatmap_upload.html",
        "config_general.html",
        "config_tuning.html",
        "config_queues.html",
        "config_lts.html",
        "config_iprange.html",
        "config_flows.html",
        "config_integration.html",
        "config_splynx.html",
        "config_netzur.html",
        "config_uisp.html",
        "config_powercode.html",
        "config_sonar.html",
        "config_interface.html",
        "config_network.html",
        "config_devices.html",
        "config_users.html",
        "config_wispgate.html",
        "config_stormguard.html",
        "stormguard_debug.html",
        "api.html",
        "cpu_weights.html",
        "cpu_tree.html",
    ];

    // Iterate through pages and construct the router
    let mut router = Router::new();
    for page in html_pages.iter() {
        let path = Path::new(&config.lqos_directory)
            .join("bin")
            .join("static2")
            .join(page);

        if !path.exists() {
            bail!("Missing webpage: {page}");
        }

        router = router.route_service(&format!("/{page}"), ServeFile::new(path));
    }

    // Backwards compatible alias for historical misspelling.
    // This route is included in the templated/authenticated router so bookmarks keep working.
    let splynx_path = Path::new(&config.lqos_directory)
        .join("bin")
        .join("static2")
        .join("config_splynx.html");
    router = router.route_service("/config_spylnx.html", ServeFile::new(splynx_path));

    router = router
        .layer(CorsLayer::very_permissive())
        .route_layer(axum::middleware::from_fn(auth_layer))
        .route_layer(axum::middleware::from_fn(apply_templates));

    Ok(router)
}
