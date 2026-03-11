use axum::body::Body;
use axum::extract::Path;
use axum::http::{HeaderMap, Request};
use axum::response::IntoResponse;
use lqos_heimdall::n_second_pcap;
use serde::Serialize;
use std::net::IpAddr;
use tower_http::services::ServeFile;

#[derive(Debug, Serialize, Clone)]
pub enum RequestAnalysisResult {
    Fail,
    Ok { session_id: usize, countdown: usize },
}

pub fn request_analysis_data(ip: &str) -> RequestAnalysisResult {
    if let Ok(ip) = ip.parse::<IpAddr>() {
        if let Some((session_id, countdown)) = lqos_heimdall::hyperfocus_on_target(ip.into()) {
            return RequestAnalysisResult::Ok {
                session_id,
                countdown,
            };
        }
    }
    RequestAnalysisResult::Fail
}

pub async fn pcap_dump(Path(id): Path<usize>, headers: HeaderMap) -> impl IntoResponse {
    let filename = n_second_pcap(id).expect("Could not determine pcap filename");
    let mut req = Request::new(Body::empty());
    *req.headers_mut() = headers;
    ServeFile::new(filename)
        .try_call(req)
        .await
        .expect("ServeFile call failed")
}
