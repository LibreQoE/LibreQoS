use axum::{Json, http::StatusCode};
use lqos_bus::{BusRequest, BusResponse, LibreqosBusClient, TopFlowType};
use lqos_utils::units::DownUpOrder;

use crate::transit_types::FlowbeeSummaryDataTransit;

#[utoipa::path(
    get,
    path = "/dump_active_flows",
    summary = "All currently tracked active flows (large payload)",
    description = "Return all currently tracked active flows (large payload).",
    responses(
        (status = 200, description = "Return 200 OK and all active flows. WARNING: This can be slow and return a lot of data!", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Flow Analysis",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn dump_active_flows()
-> Result<(StatusCode, Json<Vec<FlowbeeSummaryDataTransit>>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::DumpActiveFlows])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::AllActiveFlows(flows) = response {
            let data = flows
                .into_iter()
                .map(FlowbeeSummaryDataTransit::from)
                .collect();
            return Ok((StatusCode::OK, Json(data)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

#[utoipa::path(
    get,
    path = "/count_active_flows",
    summary = "Count of currently tracked active flows",
    description = "Return the count of currently tracked active flows.",
    responses(
        (status = 200, description = "Return 200 OK and the count of active flows.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Flow Analysis",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn count_active_flows() -> Result<(StatusCode, Json<u64>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::CountActiveFlows])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::CountActiveFlows(count) = response {
            return Ok((StatusCode::OK, Json(count)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

#[utoipa::path(
    get,
    path = "/top_flows/{flow_type}/{n}",
    summary = "Top N flows for a selected metric",
    description = "Return the top N flows for a selected metric.",
    responses(
        (status = 200, description = "Return 200 OK and top flows.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Flow Analysis",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
        ("flow_type" = String, Path, description = "The type of top flows to return: RateEstimate, Bytes, Packets, Drops, RoundTripTime"),
        ("n" = u32, Path, description = "The number of top flows to return."),
    ),
)]
pub async fn top_flows(
    axum::extract::Path((flow_type_str, n)): axum::extract::Path<(String, u32)>,
) -> Result<(StatusCode, Json<Vec<FlowbeeSummaryDataTransit>>), (StatusCode, String)> {
    let flow_type = match flow_type_str.as_str() {
        "RateEstimate" => TopFlowType::RateEstimate,
        "Bytes" => TopFlowType::Bytes,
        "Packets" => TopFlowType::Packets,
        "Drops" => TopFlowType::Drops,
        "RoundTripTime" => TopFlowType::RoundTripTime,
        _ => return Err((StatusCode::BAD_REQUEST, "Invalid flow type. Valid types are: RateEstimate, Bytes, Packets, Drops, RoundTripTime".to_string())),
    };

    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::TopFlows { flow_type, n }])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::TopFlows(flows) = response {
            let result = flows
                .into_iter()
                .map(FlowbeeSummaryDataTransit::from)
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
    path = "/flows_by_ip/{ip}",
    summary = "Flows associated with a specific IP address",
    description = "Return flows associated with a specific IP address.",
    responses(
        (status = 200, description = "Return 200 OK and flows for the specified IP.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Flow Analysis",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
        ("ip" = String, Path, description = "The IP address to get flows for."),
    ),
)]
pub async fn flows_by_ip(
    axum::extract::Path(ip): axum::extract::Path<String>,
) -> Result<(StatusCode, Json<Vec<FlowbeeSummaryDataTransit>>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::FlowsByIp(ip)])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::FlowsByIp(flows) = response {
            let flows = flows
                .into_iter()
                .map(FlowbeeSummaryDataTransit::from)
                .collect();
            return Ok((StatusCode::OK, Json(flows)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

#[utoipa::path(
    get,
    path = "/current_endpoints_by_country",
    summary = "Current endpoints grouped by country",
    description = "Return current endpoints grouped by country.",
    responses(
        (status = 200, description = "Return 200 OK and endpoints grouped by country.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Endpoint Metrics",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn current_endpoints_by_country() -> Result<
    (
        StatusCode,
        Json<Vec<(String, DownUpOrder<u64>, [f32; 2], String)>>,
    ),
    (StatusCode, String),
> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::CurrentEndpointsByCountry])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::CurrentEndpointsByCountry(data) = response {
            return Ok((StatusCode::OK, Json(data)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

#[utoipa::path(
    get,
    path = "/current_endpoint_latlon",
    summary = "Current endpoints with latitude/longitude samples",
    description = "Return current endpoints with latitude/longitude samples.",
    responses(
        (status = 200, description = "Return 200 OK and endpoint latitude/longitude data.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Endpoint Metrics",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn current_endpoint_latlon()
-> Result<(StatusCode, Json<Vec<(f64, f64, String, u64, f32)>>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::CurrentEndpointLatLon])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::CurrentLatLon(data) = response {
            return Ok((StatusCode::OK, Json(data)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

#[utoipa::path(
    get,
    path = "/flow_duration",
    summary = "Summary of flow durations",
    description = "Return summary of flow durations.",
    responses(
        (status = 200, description = "Return 200 OK and flow duration data.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Flow Analysis",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn flow_duration() -> Result<(StatusCode, Json<Vec<(usize, u64)>>), (StatusCode, String)>
{
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::FlowDuration])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::FlowDuration(data) = response {
            return Ok((StatusCode::OK, Json(data)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}
