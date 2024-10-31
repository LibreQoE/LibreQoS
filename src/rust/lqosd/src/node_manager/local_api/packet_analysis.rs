use std::net::IpAddr;
use axum::body::Body;
use axum::extract::Path;
use axum::http::{HeaderMap, Request};
use axum::Json;
use axum::response::IntoResponse;
use serde::Serialize;
use tower_http::services::ServeFile;
use lqos_heimdall::n_second_pcap;

#[derive(Serialize, Clone)]
pub enum RequestAnalysisResult {
    Fail,
    Ok{ session_id: usize, countdown: usize }
}

pub async fn request_analysis(Path(ip): Path<String>) -> Json<RequestAnalysisResult> {
    if let Ok(ip) = ip.parse::<IpAddr>() {
        if let Some((session_id, countdown)) = lqos_heimdall::hyperfocus_on_target(ip.into()) {
            return Json(RequestAnalysisResult::Ok{ session_id, countdown });
        }
    }
    Json(RequestAnalysisResult::Fail)
}

pub async fn pcap_dump(Path(id): Path<usize>, headers: HeaderMap) -> impl IntoResponse {
    let filename = n_second_pcap(id).unwrap();
    let mut req = Request::new(Body::empty());
    *req.headers_mut() = headers;
    ServeFile::new(filename).try_call(req).await.unwrap()
}