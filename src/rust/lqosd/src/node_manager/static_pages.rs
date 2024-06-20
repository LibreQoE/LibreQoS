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
    let static_path = Path::new(&config.lqos_directory)
        .join("bin")
        .join("static2");

    let index = Path::new(&config.lqos_directory)
        .join("bin")
        .join("static2")
        .join("index.html");

    if !static_path.exists() {
        bail!("Static path not found for webserver (vin/static2/");
    }

    let router = Router::new()
        .nest_service("/index.html", ServeFile::new(index.clone()))
        .nest_service("/", ServeDir::new(static_path))
        .route_layer(axum::middleware::from_fn(apply_templates));


    Ok(router)
}