use crate::errors::UispIntegrationError;
use csv::ReaderBuilder;
use lqos_config::Config;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tracing::{error, info};
use crate::uisp_types::UispSite;

pub type BandwidthOverrides = HashMap<String, (f32, f32)>;

/// Attempts to load integrationUISPbandwidths.csv to use for
/// bandwidth overrides. Returns an empty set if not found.
pub fn get_site_bandwidth_overrides(
    config: &Config,
) -> Result<BandwidthOverrides, UispIntegrationError> {
    info!("Looking for integrationUISPbandwidths.csv");
    let file_path = Path::new(&config.lqos_directory).join("integrationUISPbandwidths.csv");
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
        for (line, result) in reader.records().enumerate() {
            if let Ok(result) = result {
                if result.len() != 3 {
                    error!("Wrong number of records on line {line}");
                    continue;
                }
                let parent_node = result[0].to_string();
                if let Some(d) = numeric_string_to_f32(&result[1]) {
                    if let Some(u) = numeric_string_to_f32(&result[2]) {
                        overrides.insert(parent_node, (d, u));
                    } else {
                        error!("Cannot parse {} as float on line {line}", &result[2]);
                    }
                } else {
                    error!("Cannot parse {} as float on line {line}", &result[1]);
                }
            } else {
                error!("Error reading integrationUISPbandwidths.csv line");
                error!("{result:?}");
            }
        }

        info!("Loaded {} bandwidth overrides", overrides.len());
        return Ok(overrides);
    }

    info!("No bandwidth overrides loaded.");
    Ok(HashMap::new())
}

fn numeric_string_to_f32(text: &str) -> Option<f32> {
    if let Ok(n) = text.parse::<f32>() {
        Some(n)
    } else if let Ok(n) = text.parse::<i64>() {
        Some(n as f32)
    } else {
        error!("Unable to parse {text} as a numeric");
        None
    }
}

pub fn apply_bandwidth_overrides(sites: &mut Vec<UispSite>, bandwidth_overrides: &BandwidthOverrides) {
    for site in sites.iter_mut() {
        if let Some((up, down)) = bandwidth_overrides.get(&site.name) {
            tracing::info!("Bandwidth override for {} applied", &site.name);
            // Apply the overrides
            site.max_down_mbps = *down as u32;
            site.max_up_mbps = *up as u32;
        }
    }
}