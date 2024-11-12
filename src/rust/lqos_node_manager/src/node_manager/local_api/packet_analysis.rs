use std::net::IpAddr;
use axum::body::Body;
use axum::extract::Path;
use axum::http::{HeaderMap, Request};
use axum::Json;
use axum::response::IntoResponse;
use serde::Serialize;
use tower_http::services::ServeFile;
use lqos_bus::{bus_request, BusRequest, BusResponse};

#[derive(Serialize, Clone)]
pub enum RequestAnalysisResult {
    Fail,
    Ok{ session_id: usize, countdown: usize }
}

pub async fn request_analysis(Path(ip): Path<String>) -> Json<RequestAnalysisResult> {
    if let Ok(ip) = ip.parse::<IpAddr>() {
        let Ok(replies) = bus_request(vec![BusRequest::GatherPacketData(ip.to_string())]).await else {
            return Json(RequestAnalysisResult::Fail);
        };
        for reply in replies {
            if let BusResponse::PacketCollectionSession { session_id, countdown } = reply {
                return Json(RequestAnalysisResult::Ok { session_id, countdown });
            }
        }
    }
    Json(RequestAnalysisResult::Fail)
}

pub async fn pcap_dump(Path(id): Path<usize>, headers: HeaderMap) -> impl IntoResponse {
    let replies = bus_request(vec![BusRequest::GetPcapDump(id)]).await.unwrap();
    let mut filename = String::new();
    for reply in replies {
        if let BusResponse::PcapDump(id) = reply {
            filename = id.unwrap();
            break;
        }
    }

    let mut req = Request::new(Body::empty());
    *req.headers_mut() = headers;
    ServeFile::new(filename).try_call(req).await.unwrap()
}