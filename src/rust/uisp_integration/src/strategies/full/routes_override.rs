use crate::errors::UispIntegrationError;
use csv::ReaderBuilder;
use lqos_config::Config;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tracing::{error, info};

/// Represents a route override in the integrationUISProutes.csv file.
#[derive(Serialize, Deserialize, Debug)]
pub struct RouteOverride {
    /// The site to override the route from.
    pub from_site: String,
    /// The site to override the route to.
    pub to_site: String,
    /// The cost of the route.
    pub cost: u32,
}

/// Attempts to load integrationUISProutes.csv to use for
/// route overrides. Returns an empty set if not found.
/// Returns an error if the file is found but cannot be read.
///
/// The file should be a CSV with the following columns:
///
/// | From Site | To Site | Cost |
/// |-----------|---------|------|
/// | Site1     | Site2   | 100  |
/// | Site2     | Site3   | 200  |
///
/// The From Site and To Site should match the name of the site in UISP.
///
/// If the file is found, the overrides will be applied to the routes
/// in the `UispSite` array by the `apply_route_overrides` function.
///
/// # Arguments
/// * `config` - The configuration
///
/// # Returns
/// * An `Ok(Vec)` of `RouteOverride` objects
/// * An `Err` if the file is found but cannot be read
pub fn get_route_overrides(config: &Config) -> Result<Vec<RouteOverride>, UispIntegrationError> {
    // Prefer overrides from lqos_overrides.json if present
    if let Ok(of) = lqos_overrides::OverrideFile::load() {
        if let Some(uisp) = of.uisp() {
            if !uisp.route_overrides.is_empty() {
                info!("Using UISP route overrides from lqos_overrides.json");
                let converted: Vec<RouteOverride> = uisp
                    .route_overrides
                    .iter()
                    .map(|r| RouteOverride {
                        from_site: r.from_site.clone(),
                        to_site: r.to_site.clone(),
                        cost: r.cost,
                    })
                    .collect();
                return Ok(converted);
            }
        }
    }

    let file_path = Path::new(&config.lqos_directory).join("integrationUISProutes.csv");
    if file_path.exists() {
        let reader = ReaderBuilder::new()
            .has_headers(false)
            .comment(Some(b'#'))
            .trim(csv::Trim::All)
            .from_path(file_path);
        if reader.is_err() {
            error!("Unable to read integrationUISProutes.csv");
            error!("{:?}", reader);
            return Err(UispIntegrationError::CsvError);
        }
        let mut reader = reader.unwrap();
        let mut overrides = Vec::new();

        for rec in reader.records() {
            if let Ok(line) = rec {
                if line.len() == 3 {
                    println!("{line:?}");
                    // We got a line
                    if let Ok(cost) = &line[2].parse::<u32>() {
                        overrides.push(RouteOverride {
                            from_site: line[0].to_string(),
                            to_site: line[1].to_string(),
                            cost: *cost,
                        });
                    } else {
                        error!("{line:?} is not a valid integer for cost");
                    }
                }
            } else {
                error!("Unable to read route overrides CSV");
                error!("{rec:?}");
            }
        }

        info!("Loaded {} route overrides", overrides.len());
        Ok(overrides)
    } else {
        info!("No integrationUISProutes.csv found - no route overrides loaded.");
        Ok(Vec::new())
    }
}

#[allow(dead_code)]
pub fn write_routing_overrides_template(
    config: Arc<Config>,
    natural_routes: &[RouteOverride],
) -> anyhow::Result<()> {
    let file_path = Path::new(&config.lqos_directory).join("integrationUISProutes.template.csv");
    let mut writer = csv::Writer::from_path(file_path)?;
    writer.write_record(&["From Site", "To Site", "Cost"])?;
    for route in natural_routes {
        writer.write_record(&[&route.from_site, &route.to_site, &route.cost.to_string()])?;
    }
    writer.flush()?;
    Ok(())
}
