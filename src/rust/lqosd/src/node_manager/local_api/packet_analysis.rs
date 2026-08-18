use axum::body::Body;
use axum::extract::Path;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::IntoResponse;
use lqos_heimdall::n_second_pcap;
use serde::Serialize;
use std::net::IpAddr;
use tower_http::services::ServeFile;
use tracing::warn;

#[derive(Debug, Serialize, Clone)]
pub enum RequestAnalysisResult {
    Fail,
    Ok { session_id: usize, countdown: usize },
}

pub fn request_analysis_data(ip: &str) -> RequestAnalysisResult {
    if let Ok(ip) = ip.parse::<IpAddr>()
        && let Some((session_id, countdown)) = lqos_heimdall::hyperfocus_on_target(ip.into())
    {
        return RequestAnalysisResult::Ok {
            session_id,
            countdown,
        };
    }
    RequestAnalysisResult::Fail
}

pub async fn pcap_dump(
    Path(id): Path<usize>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    let Some(filename) = n_second_pcap(id) else {
        return Err(StatusCode::NOT_FOUND);
    };

    let mut req = Request::new(Body::empty());
    *req.headers_mut() = headers;
    match ServeFile::new(&filename).try_call(req).await {
        Ok(response) => Ok(response),
        Err(err) => {
            warn!("Unable to serve packet capture file {filename}: {err}");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pcap_dump_returns_not_found_for_missing_session() {
        let response = pcap_dump(Path(usize::MAX), HeaderMap::new())
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
