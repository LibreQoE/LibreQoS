use std::path::Path;
use axum::Router;
use tower_http::services::{ServeDir, ServeFile};
use lqos_config::load_config;
use anyhow::{bail, Result};
use crate::node_manager::template::apply_templates;

pub(super) fn vendor_route() -> Result<Router> {
    let config = load_config()?;
    let vendor_path = Path::new(&config.lqos_directory)
        .join("bin")
        .join("static2")
        .join("vendor");

    if !vendor_path.exists() {
        bail!("Vendor path not found for webserver (bin/static2/vendor/");
    }

    let router = Router::new()
        .nest_service("/", ServeDir::new(vendor_path));

    Ok(router)
}

pub(super) fn static_routes() -> Result<Router> {
    let config = load_config()?;
    
    // Add HTML pages to serve directly to this list, otherwise
    // they won't have template + authentication applied to them.
    let html_pages = [
        "index.html"
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
    router = router
        .route_layer(axum::middleware::from_fn(apply_templates));

    Ok(router)
}