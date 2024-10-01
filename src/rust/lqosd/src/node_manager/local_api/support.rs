use std::time::{SystemTime, UNIX_EPOCH};
use axum::body::Body;
use axum::http::header;
use axum::Json;
use axum::response::IntoResponse;
use serde::Deserialize;
use lqos_config::load_config;
use lqos_support_tool::{run_sanity_checks, SanityChecks};

pub async fn run_sanity_check() -> Json<SanityChecks> {
    let mut status = run_sanity_checks(false).unwrap();
    status.results.sort_by(|a,b| a.success.cmp(&b.success));
    Json(status)
}

#[derive(Deserialize, Clone)]
pub struct SupportMetadata {
    name: String,
    comment: String,
}

pub async fn gather_support_data(
    info: Json<SupportMetadata>
) -> impl IntoResponse {
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let filename = format!("libreqos_{}.support", timestamp);
    let lts_key = if let Ok(cfg) = load_config() {
        cfg.long_term_stats.license_key.unwrap_or("None".to_string())
    } else {
        "None".to_string()
    };
    let dump = lqos_support_tool::gather_all_support_info(&info.name, &info.comment, &lts_key).unwrap();

    let body = Body::from(dump.serialize_and_compress().unwrap());
    let headers = [
        (header::CONTENT_TYPE, "application/octet-stream"),
        (
            header::CONTENT_DISPOSITION,
            &format!("attachment; filename=\"{filename}\""),
        ),
    ];
    (headers, body).into_response()
}

pub async fn submit_support_data(
    info: Json<SupportMetadata>
) -> String {
    let lts_key = if let Ok(cfg) = load_config() {
        cfg.long_term_stats.license_key.unwrap_or("None".to_string())
    } else {
        "None".to_string()
    };
    if let Ok(dump) = lqos_support_tool::gather_all_support_info(&info.name, &info.comment, &lts_key) {
        lqos_support_tool::submit_to_network(dump);
        "Your support submission has been sent".to_string()
    } else {
        "Something went wrong".to_string()
    }

}