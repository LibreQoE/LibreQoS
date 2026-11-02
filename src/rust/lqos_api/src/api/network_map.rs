use axum::{Json, http::StatusCode};
use lqos_bus::{BusRequest, BusResponse, LibreqosBusClient};

use crate::transit_types::{CircuitTransit, NetworkJsonTransit};

#[utoipa::path(
    get,
    path = "/network_map/{parent}",
    summary = "Partial network map for a parent node ID",
    description = "Return a partial network map for a parent node ID.",
    responses(
        (status = 200, description = "Return 200 OK and network map data.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Network Map",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
        ("parent" = usize, Path, description = "The parent node ID to get the network map for."),
    ),
)]
pub async fn get_network_map(
    axum::extract::Path(parent): axum::extract::Path<usize>,
) -> Result<(StatusCode, Json<Vec<(usize, NetworkJsonTransit)>>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetNetworkMap { parent }])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::NetworkMap(data) = response {
            let result = data
                .into_iter()
                .map(|q| (q.0, NetworkJsonTransit::from(q.1)))
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
    path = "/full_network_map",
    summary = "Full network map for the current topology",
    description = "Return the full network map for the current topology.",
    responses(
        (status = 200, description = "Return 200 OK and full network map data.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Network Map",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn get_full_network_map()
-> Result<(StatusCode, Json<Vec<(usize, NetworkJsonTransit)>>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetFullNetworkMap])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::NetworkMap(data) = response {
            let result = data
                .into_iter()
                .map(|q| (q.0, NetworkJsonTransit::from(q.1)))
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
    path = "/network_tree",
    summary = "Network tree snapshot for the current topology",
    description = "Return the full network tree snapshot for the current topology.",
    responses(
        (status = 200, description = "Return 200 OK and full network tree data.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Network Map",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn get_network_tree()
-> Result<(StatusCode, Json<Vec<(usize, NetworkJsonTransit)>>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetFullNetworkMap])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::NetworkMap(data) = response {
            let result = data
                .into_iter()
                .map(|q| (q.0, NetworkJsonTransit::from(q.1)))
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
    path = "/network_node/{node_name}",
    summary = "Network node entry by name",
    description = "Return the network map entry matching a node name.",
    responses(
        (status = 200, description = "Return 200 OK and the matching network map entry.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 404, description = "Return 404 Not Found if the node name is not found.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Monitoring",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
        ("node_name" = String, Path, description = "The node name to look up.", example = "example_site"),
    ),
)]
pub async fn get_network_node_by_name(
    axum::extract::Path(node_name): axum::extract::Path<String>,
) -> Result<(StatusCode, Json<Vec<(usize, NetworkJsonTransit)>>), (StatusCode, String)> {
    let node_name_lower = node_name.to_ascii_lowercase();
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetFullNetworkMap])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::NetworkMap(data) = response {
            let filtered = data
                .into_iter()
                .map(|q| (q.0, NetworkJsonTransit::from(q.1)))
                .filter(|(_, node)| node.name.to_ascii_lowercase() == node_name_lower)
                .collect::<Vec<_>>();
            if filtered.is_empty() {
                return Err((
                    StatusCode::NOT_FOUND,
                    format!("No network node found for '{}'", node_name),
                ));
            }
            return Ok((StatusCode::OK, Json(filtered)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

#[utoipa::path(
    get,
    path = "/top_map_queues/{n}",
    summary = "Top N queues from the network map for dashboard views",
    description = "Return the top N queues from the network map for dashboard views.",
    responses(
        (status = 200, description = "Return 200 OK and top map queues data.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Network Map",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
        ("n" = usize, Path, description = "The number of top queues to return."),
    ),
)]
pub async fn top_map_queues(
    axum::extract::Path(n): axum::extract::Path<usize>,
) -> Result<(StatusCode, Json<Vec<(usize, NetworkJsonTransit)>>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::TopMapQueues(n)])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::NetworkMap(data) = response {
            let data = data
                .into_iter()
                .map(|q| (q.0, NetworkJsonTransit::from(q.1)))
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
    post,
    path = "/node_names",
    summary = "Resolve node IDs to their names",
    description = "Resolve node IDs to their names.",
    responses(
        (status = 200, description = "Return 200 OK and node names.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Network Map",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
    request_body = Vec<usize>,
)]
pub async fn get_node_names_from_ids(
    Json(ids): Json<Vec<usize>>,
) -> Result<(StatusCode, Json<Vec<(usize, String)>>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetNodeNamesFromIds(ids)])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::NodeNames(names) = response {
            return Ok((StatusCode::OK, Json(names)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

#[utoipa::path(
    get,
    path = "/all_circuits",
    summary = "List of all circuits known to lqosd",
    description = "Return the list of all circuits known to lqosd.",
    responses(
        (status = 200, description = "Return 200 OK and all circuits data.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Network Map",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn get_all_circuits()
-> Result<(StatusCode, Json<Vec<CircuitTransit>>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetAllCircuits])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::CircuitData(circuits) = response {
            let circuits = circuits.into_iter().map(CircuitTransit::from).collect();
            return Ok((StatusCode::OK, Json(circuits)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}

#[utoipa::path(
    get,
    path = "/funnel/{target}",
    summary = "Funnel data for a circuit ID",
    description = "Return funnel data for a circuit ID.",
    responses(
        (status = 200, description = "Return 200 OK and funnel data for the circuit.", body = String),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
        (status = 403, description = "Return 403 Forbidden if the current LibreQoS API policy denies access.", body = String),
        (status = 500, description = "Return 500 Internal Server Error if the lqosd bus is not available (maybe lqosd isn't running?).", body = String),
    ),
    tag = "Network Map",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
        ("target" = String, Path, description = "The circuit ID to analyze."),
    ),
)]
pub async fn get_funnel(
    axum::extract::Path(target): axum::extract::Path<String>,
) -> Result<(StatusCode, Json<Vec<(usize, NetworkJsonTransit)>>), (StatusCode, String)> {
    let mut client = LibreqosBusClient::new().await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to connect to bus. Is LibreQoS Running?".to_string(),
        )
    })?;

    let responses = client
        .request(vec![BusRequest::GetFunnel { target }])
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to communicate with bus: {}", e),
            )
        })?;
    for response in responses {
        if let BusResponse::NetworkMap(data) = response {
            let result = data
                .into_iter()
                .map(|q| (q.0, NetworkJsonTransit::from(q.1)))
                .collect();
            return Ok((StatusCode::OK, Json(result)));
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unexpected response from bus.".to_string(),
    ))
}
