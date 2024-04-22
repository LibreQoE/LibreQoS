use std::path::Path;
use csv::ReaderBuilder;
use serde::Deserialize;
use tracing::{error, info};
use lqos_config::Config;
use crate::errors::UispIntegrationError;

#[derive(Deserialize, Debug)]
pub struct RouteOverride {
    pub from_site: String,
    pub to_site: String,
    pub cost: u32,
}

pub fn get_route_overrides(config: &Config) -> Result<Vec<RouteOverride>, UispIntegrationError> {
    let file_path = Path::new(&config.lqos_directory).join("integrationUISProutes.csv");
    if file_path.exists() {
        let reader = ReaderBuilder::new()
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
        for result in reader.deserialize::<RouteOverride>() {
            if let Ok(result) = result {
                overrides.push(result);
            }
        }
        info!("Loaded {} route overrides", overrides.len());
        Ok(overrides)
    } else {
        info!("No integrationUISProutes.csv found - no route overrides loaded.");
        Ok(Vec::new())
    }
}