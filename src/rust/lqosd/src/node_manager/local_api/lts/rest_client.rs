//! Provides a helpful wrapper to Reqwest with the
//! appropriate settings enabled for the Insight API.

use axum::http::StatusCode;
use lqos_config::load_config;
use tracing::error;

pub(crate) async fn lts_query<T>(url: &str) -> Result<Vec<T>, StatusCode>
where
    T: serde::de::DeserializeOwned,
{
    let config = load_config().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(|e| {
            error!("Error building reqwest client: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let received_data = client
        .get(url)
        .header(
            "x-license-key",
            config
                .long_term_stats
                .license_key
                .clone()
                .unwrap_or("".to_string()),
        )
        .header("x-node-id", config.node_id.to_string())
        .send()
        .await
        .map_err(|e| {
            error!("Error getting throughput data: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .json::<Vec<T>>()
        .await
        .map_err(|e| {
            error!("Error parsing throughput data: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(received_data)
}
