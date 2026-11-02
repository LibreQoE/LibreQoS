use axum::{Json, http::StatusCode};
use lqos_bus::{BusRequest, BusResponse, LibreqosBusClient};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::transit_types::DownUpTransit;

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct FlowMapPointData {
    pub lat: f64,
    pub lon: f64,
    pub country: String,
    pub bytes_sent: u64,
    pub rtt_nanos: f32,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct AsnListEntryData {
    pub count: usize,
    pub asn: u32,
    pub name: String,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct CountryListEntryData {
    pub count: usize,
    pub name: String,
    pub iso_code: String,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct ProtocolListEntryData {
    pub count: usize,
    pub protocol: String,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct FlowTimelineEntryData {
    pub start: u64,
    pub end: u64,
    pub duration_nanos: u64,
    pub throughput: Vec<DownUpTransit>,
    pub tcp_retransmits: DownUpTransit,
    pub rtt_nanos: [u64; 2],
    pub retransmit_times_down: Vec<u64>,
    pub retransmit_times_up: Vec<u64>,
    pub total_bytes: DownUpTransit,
    pub protocol: String,
    pub circuit_id: String,
    pub circuit_name: String,
    pub remote_ip: String,
}

#[utoipa::path(
    get,
    path = "/flow_map",
    summary = "Flow map endpoints",
    description = "Return recent flow endpoints as latitude/longitude points.",
    responses(
        (status = 200, description = "Return 200 OK and flow map points.", body = Vec<FlowMapPointData>),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Flow Analysis",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn flow_map() -> Result<(StatusCode, Json<Vec<FlowMapPointData>>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetFlowMap])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::FlowMap(points) = response {
            let result = points
                .into_iter()
                .map(|p| FlowMapPointData {
                    lat: p.lat,
                    lon: p.lon,
                    country: p.country,
                    bytes_sent: p.bytes_sent,
                    rtt_nanos: p.rtt_nanos,
                })
                .collect();
            return Ok((StatusCode::OK, Json(result)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

#[utoipa::path(
    get,
    path = "/flow_explorer/asns",
    summary = "Flow explorer ASN list",
    description = "Return recent ASN list with flow counts.",
    responses(
        (status = 200, description = "Return 200 OK and ASN list.", body = Vec<AsnListEntryData>),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Flow Analysis",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn flow_explorer_asns()
-> Result<(StatusCode, Json<Vec<AsnListEntryData>>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetAsnList])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::AsnList(list) = response {
            let result = list
                .into_iter()
                .map(|entry| AsnListEntryData {
                    count: entry.count,
                    asn: entry.asn,
                    name: entry.name,
                })
                .collect();
            return Ok((StatusCode::OK, Json(result)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

#[utoipa::path(
    get,
    path = "/flow_explorer/countries",
    summary = "Flow explorer country list",
    description = "Return recent country list with flow counts.",
    responses(
        (status = 200, description = "Return 200 OK and country list.", body = Vec<CountryListEntryData>),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Flow Analysis",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn flow_explorer_countries()
-> Result<(StatusCode, Json<Vec<CountryListEntryData>>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetCountryList])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::CountryList(list) = response {
            let result = list
                .into_iter()
                .map(|entry| CountryListEntryData {
                    count: entry.count,
                    name: entry.name,
                    iso_code: entry.iso_code,
                })
                .collect();
            return Ok((StatusCode::OK, Json(result)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

#[utoipa::path(
    get,
    path = "/flow_explorer/protocols",
    summary = "Flow explorer protocol list",
    description = "Return recent protocol list with flow counts.",
    responses(
        (status = 200, description = "Return 200 OK and protocol list.", body = Vec<ProtocolListEntryData>),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Flow Analysis",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn flow_explorer_protocols()
-> Result<(StatusCode, Json<Vec<ProtocolListEntryData>>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetProtocolList])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::ProtocolList(list) = response {
            let result = list
                .into_iter()
                .map(|entry| ProtocolListEntryData {
                    count: entry.count,
                    protocol: entry.protocol,
                })
                .collect();
            return Ok((StatusCode::OK, Json(result)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

#[utoipa::path(
    get,
    path = "/flow_explorer/asn/{asn}",
    summary = "Flow explorer ASN timeline",
    description = "Return flow timeline entries for an ASN.",
    responses(
        (status = 200, description = "Return 200 OK and ASN flow timeline.", body = Vec<FlowTimelineEntryData>),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Flow Analysis",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
        ("asn" = u32, Path, description = "ASN number to filter.", example = 13335),
    ),
)]
pub async fn flow_explorer_asn_timeline(
    axum::extract::Path(asn): axum::extract::Path<u32>,
) -> Result<(StatusCode, Json<Vec<FlowTimelineEntryData>>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetAsnFlowTimeline { asn }])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        // v3 also returns `buckets`; the v1 API surface deliberately returns
        // `entries` only (documented in the manual and changelog).
        if let BusResponse::AsnFlowTimeline { entries, .. } = response {
            let result = entries.into_iter().map(flow_timeline_to_transit).collect();
            return Ok((StatusCode::OK, Json(result)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

#[utoipa::path(
    get,
    path = "/flow_explorer/country/{iso}",
    summary = "Flow explorer country timeline",
    description = "Return flow timeline entries for a country ISO code.",
    responses(
        (status = 200, description = "Return 200 OK and country flow timeline.", body = Vec<FlowTimelineEntryData>),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Flow Analysis",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
        ("iso" = String, Path, description = "ISO country code to filter.", example = "US"),
    ),
)]
pub async fn flow_explorer_country_timeline(
    axum::extract::Path(iso): axum::extract::Path<String>,
) -> Result<(StatusCode, Json<Vec<FlowTimelineEntryData>>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetCountryFlowTimeline { iso_code: iso }])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        // v3 also returns `buckets`; the v1 API surface deliberately returns
        // `entries` only (documented in the manual and changelog).
        if let BusResponse::CountryFlowTimeline { entries, .. } = response {
            let result = entries.into_iter().map(flow_timeline_to_transit).collect();
            return Ok((StatusCode::OK, Json(result)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

#[utoipa::path(
    get,
    path = "/flow_explorer/protocol/{protocol}",
    summary = "Flow explorer protocol timeline",
    description = "Return flow timeline entries for a protocol.",
    responses(
        (status = 200, description = "Return 200 OK and protocol flow timeline.", body = Vec<FlowTimelineEntryData>),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Flow Analysis",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
        ("protocol" = String, Path, description = "Protocol name to filter.", example = "TCP"),
    ),
)]
pub async fn flow_explorer_protocol_timeline(
    axum::extract::Path(protocol): axum::extract::Path<String>,
) -> Result<(StatusCode, Json<Vec<FlowTimelineEntryData>>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetProtocolFlowTimeline { protocol }])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        // v3 also returns `buckets`; the v1 API surface deliberately returns
        // `entries` only (documented in the manual and changelog).
        if let BusResponse::ProtocolFlowTimeline { entries, .. } = response {
            let result = entries.into_iter().map(flow_timeline_to_transit).collect();
            return Ok((StatusCode::OK, Json(result)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

fn flow_timeline_to_transit(entry: lqos_bus::FlowTimelineEntry) -> FlowTimelineEntryData {
    FlowTimelineEntryData {
        start: entry.start,
        end: entry.end,
        duration_nanos: entry.duration_nanos,
        throughput: entry
            .throughput
            .into_iter()
            .map(DownUpTransit::from)
            .collect(),
        tcp_retransmits: DownUpTransit::from(entry.tcp_retransmits),
        rtt_nanos: entry.rtt_nanos,
        retransmit_times_down: entry.retransmit_times_down,
        retransmit_times_up: entry.retransmit_times_up,
        total_bytes: DownUpTransit::from(entry.total_bytes),
        protocol: entry.protocol,
        circuit_id: entry.circuit_id,
        circuit_name: entry.circuit_name,
        remote_ip: entry.remote_ip,
    }
}
