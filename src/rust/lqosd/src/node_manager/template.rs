//! Provides an Axum layer that applies templates to static HTML
//! files.

use std::path::Path;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderValue, Request, Response, StatusCode};
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum_extra::extract::CookieJar;
use lqos_config::load_config;
use crate::node_manager::auth::get_username;

const VERSION_STRING: &str = include_str!("../../../../VERSION_STRING");

pub async fn apply_templates(
    jar: CookieJar,
    req: Request<axum::body::Body>,
    next: Next,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let apply_template = {
        let path = &req.uri().path().to_string();
        path.ends_with(".html")
    };

    // TODO: Cache this once we're not continually making changes
    let template_text = {
        let config = load_config().unwrap();
        let path = Path::new(&config.lqos_directory)
            .join("bin")
            .join("static2")
            .join("template.html");
        std::fs::read_to_string(path).unwrap()
    };

    // Update the displayed username
    let username = get_username(&jar).await;
    let template_text = template_text.replace("%%USERNAME%%", &username);

    let res = next.run(req).await;

    if apply_template {
        let (mut res_parts, res_body) = res.into_parts();
        let bytes = to_bytes(res_body, 1_000_000).await.unwrap();
        let byte_string = String::from_utf8_lossy(&bytes).to_string();
        let byte_string = template_text
            .replace("%%BODY%%", &byte_string)
            .replace("%%VERSION%%", VERSION_STRING);
        if let Some(length) = res_parts.headers.get_mut("content-length") {
            *length = HeaderValue::from(byte_string.len());
        }
        let res = Response::from_parts(res_parts, Body::from(byte_string));
        Ok(res)
    } else {
        Ok(res)
    }
}