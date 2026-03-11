use crate::errors::UispIntegrationError;
use crate::uisp_types::UispSite;
use csv::ReaderBuilder;
use lqos_config::Config;
use std::collections::HashMap;
use std::path::Path;
use tracing::{error, info};

pub type BandwidthOverrides = HashMap<String, (f32, f32)>;

/// Attempts to load integrationUISPbandwidths.csv to use for
/// bandwidth overrides. Returns an empty set if not found.
/// Returns an error if the file is found but cannot be read.
///
/// The file should be a CSV with the following columns:
///
/// | Parent Node | Down | Up |
/// |-------------|------|----|
/// | Site1       | 100  | 10 |
/// | Site2       | 200  | 20 |
///
/// The Parent Node should match the name of the site in UISP.
/// The Down and Up columns should be the desired bandwidth in Mbps.
///
/// If the file is found, the overrides will be applied to the sites
/// in the `UispSite` array by the `apply_bandwidth_overrides` function.
///
/// # Arguments
/// * `config` - The configuration
///
/// # Returns
/// * A `BandwidthOverrides` map of site names to bandwidth overrides
pub fn get_site_bandwidth_overrides(
    config: &Config,
) -> Result<BandwidthOverrides, UispIntegrationError> {
    // Prefer overrides from lqos_overrides.json if present
    if let Ok(of) = lqos_overrides::OverrideFile::load() {
        if let Some(uisp) = of.uisp() {
            if !uisp.bandwidth_overrides.is_empty() {
                info!("Using UISP bandwidth overrides from lqos_overrides.json");
                return Ok(uisp.bandwidth_overrides.clone());
            }
        }
    }

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
                        info!("Loaded bandiwdth override: {}, {}/{}", parent_node, d, u);
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

/// Applies the bandwidth overrides to the sites in the array.
///
/// # Arguments
/// * `sites` - The list of sites to modify
/// * `bandwidth_overrides` - The bandwidth overrides to apply
pub fn apply_bandwidth_overrides(sites: &mut [UispSite], bandwidth_overrides: &BandwidthOverrides) {
    for site in sites.iter_mut() {
        if let Some((down, up)) = bandwidth_overrides.get(&site.name) {
            // Apply the overrides
            site.max_down_mbps = *down as u64;
            site.max_up_mbps = *up as u64;
            info!(
                "Bandwidth override for {} applied ({} / {})",
                &site.name, site.max_down_mbps, site.max_up_mbps
            );
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_numeric_string_to_f32_valid_float() {
        let result = numeric_string_to_f32("3.2");
        assert_eq!(result, Some(3.2));
    }

    #[test]
    fn test_numeric_string_to_f32_valid_integer() {
        let result = numeric_string_to_f32("42");
        assert_eq!(result, Some(42.0));
    }

    #[test]
    fn test_numeric_string_to_f32_invalid_string() {
        let result = numeric_string_to_f32("abc");
        assert_eq!(result, None);
    }
}
