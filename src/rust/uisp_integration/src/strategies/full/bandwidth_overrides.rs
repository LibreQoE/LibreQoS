use std::collections::HashMap;
use std::path::Path;
use csv::ReaderBuilder;
use tracing::{error, info};
use serde::{Deserialize, Serialize};
use crate::errors::UispIntegrationError;

pub type BandwidthOverrides = HashMap<String, (f32, f32)>;

/// Attempts to load integrationUISPbandwidths.csv to use for
/// bandwidth overrides. Returns an empty set if not found.
pub fn get_site_bandwidth_overrides() -> Result<BandwidthOverrides, UispIntegrationError> {
    info!("Looking for integrationUISPbandwidths.csv");
    let file_path = Path::new("integrationUISPbandwidths.csv");
    if file_path.exists() {
        let reader = ReaderBuilder::new()
            .comment(Some(b'#'))
            .trim(csv::Trim::All)
            .from_path(file_path);
        if reader.is_err() {
            error!("Unable to read integrationUISPbandwidths.csv");
            error!("{:?}", reader);
            return Err(UispIntegrationError::CsvError);
        }
        let mut reader = reader.unwrap();
        let mut overrides = HashMap::new();
        for result in reader.deserialize::<IntegrationBandwidthRow>() {
            if let Ok(result) = result {
                overrides.insert(
                    result.parent_node,
                    (result.download_mbps, result.upload_mbps)
                );
            }
        }
        info!("Loaded {} bandwidth overrides", overrides.len());
        return Ok(overrides);
    }

    info!("No bandwidth overrides loaded.");
    Ok(HashMap::new())
}

#[derive(Serialize, Deserialize)]
struct IntegrationBandwidthRow {
    #[serde(rename = "ParentNode")]
    pub parent_node: String,
    #[serde(rename = "Download Mbs")]
    pub download_mbps: f32,
    #[serde(rename = "Upload Mbps")]
    pub upload_mbps: f32,
}