use axum::{Json, extract::Path, http::StatusCode};
use lqos_bus::{BusRequest, BusResponse, LibreqosBusClient};

use crate::transit_types::QooDataTransit;

async fn fetch_qoo(
    request: BusRequest,
    not_found: String,
) -> Result<QooDataTransit, (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client.request(vec![request]).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to communicate with bus: {}", e),
        )
    })?;
    for response in responses {
        if let BusResponse::Qoo(data) = response {
            let Some(data) = data else {
                return Err((StatusCode::NOT_FOUND, not_found));
            };
            return Ok(QooDataTransit::from(data));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

#[utoipa::path(
    get,
    path = "/qoo",
    summary = "Top-level QoO history",
    description = "Return the top-level QoO history displayed on the dashboard.",
    responses(
        (status = 200, description = "Top-level QoO history.", body = QooDataTransit),
        (status = 401, description = "Return 401 Unauthorized if the license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the license is invalid or not checked yet.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available.", body = String),
    ),
    tag = "Monitoring",
    params(
        ("x-bearer" = String, Header, description = "The license key to use for authentication.", example = "YOUR_INSIGHT_LICENSE_KEY_HERE"),
    ),
)]
pub async fn qoo() -> Result<(StatusCode, Json<QooDataTransit>), (StatusCode, String)> {
    let data = fetch_qoo(
        BusRequest::GetQoo,
        "No top-level QoO data found".to_string(),
    )
    .await?;
    Ok((StatusCode::OK, Json(data)))
}

#[utoipa::path(
    get,
    path = "/qoo/site/{site_name}",
    summary = "Site QoO history",
    description = "Return the dashboard QoO history for a site name.",
    responses(
        (status = 200, description = "Site QoO history.", body = QooDataTransit),
        (status = 401, description = "Return 401 Unauthorized if the license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the license is invalid or not checked yet.", body = String),
        (status = 404, description = "Return 404 Not Found if the site has no QoO data.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available.", body = String),
    ),
    tag = "Monitoring",
    params(
        ("x-bearer" = String, Header, description = "The license key to use for authentication.", example = "YOUR_INSIGHT_LICENSE_KEY_HERE"),
        ("site_name" = String, Path, description = "Site name to query.", example = "example_site"),
    ),
)]
pub async fn qoo_site(
    Path(site_name): Path<String>,
) -> Result<(StatusCode, Json<QooDataTransit>), (StatusCode, String)> {
    let not_found = format!("No QoO data found for site '{}'", site_name);
    let data = fetch_qoo(BusRequest::GetSiteQoo { site_name }, not_found).await?;
    Ok((StatusCode::OK, Json(data)))
}

#[utoipa::path(
    get,
    path = "/qoo/circuit/{circuit_id}",
    summary = "Circuit QoO history",
    description = "Return the dashboard QoO history for a circuit ID.",
    responses(
        (status = 200, description = "Circuit QoO history.", body = QooDataTransit),
        (status = 401, description = "Return 401 Unauthorized if the license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the license is invalid or not checked yet.", body = String),
        (status = 404, description = "Return 404 Not Found if the circuit has no QoO data.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available.", body = String),
    ),
    tag = "Monitoring",
    params(
        ("x-bearer" = String, Header, description = "The license key to use for authentication.", example = "YOUR_INSIGHT_LICENSE_KEY_HERE"),
        ("circuit_id" = String, Path, description = "Circuit ID to query.", example = "circuit123"),
    ),
)]
pub async fn qoo_circuit(
    Path(circuit_id): Path<String>,
) -> Result<(StatusCode, Json<QooDataTransit>), (StatusCode, String)> {
    let not_found = format!("No QoO data found for circuit '{}'", circuit_id);
    let data = fetch_qoo(BusRequest::GetCircuitQoo { circuit_id }, not_found).await?;
    Ok((StatusCode::OK, Json(data)))
}
