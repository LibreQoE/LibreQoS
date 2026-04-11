use crate::errors::UispIntegrationError;
use lqos_config::Config;
use lqos_overrides::OverrideFile;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{info, warn};

const LEGACY_UISP_ROUTES_FILE: &str = "integrationUISProutes.csv";

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

/// Attempts to load UISP route overrides.
///
/// UISP route overrides are deprecated and ignored by current UISP builds.
pub fn get_route_overrides(config: &Config) -> Result<Vec<RouteOverride>, UispIntegrationError> {
    let operator_path = OverrideFile::operator_path_for_config(config);
    match load_operator_override_file(&operator_path) {
        Ok(operator_overrides) => {
            let configured = materialize_uisp_route_overrides(&operator_overrides);
            if !configured.is_empty() {
                warn!(
                    path = %operator_path.display(),
                    count = configured.len(),
                    "UISP route overrides are deprecated and ignored; use Topology Manager parent or attachment preferences instead"
                );
            }
        }
        Err(err) if operator_path.exists() => {
            warn!(
                path = %operator_path.display(),
                "Unable to inspect operator overrides for deprecated UISP route overrides: {err:?}"
            );
        }
        Err(_) => {}
    }

    let legacy_path = legacy_routes_csv_path(config);
    if legacy_path.exists() {
        warn!(
            path = %legacy_path.display(),
            "Legacy integrationUISProutes.csv is deprecated and ignored; use Topology Manager parent or attachment preferences instead"
        );
    }

    info!(
        "UISP route overrides are disabled; using detected topology and Topology Manager overrides only."
    );
    Ok(Vec::new())
}

fn load_operator_override_file(operator_path: &Path) -> anyhow::Result<OverrideFile> {
    if !operator_path.exists() {
        return Ok(OverrideFile::default());
    }
    OverrideFile::load_from_explicit_path(operator_path)
}

fn legacy_routes_csv_path(config: &Config) -> PathBuf {
    Path::new(&config.lqos_directory).join(LEGACY_UISP_ROUTES_FILE)
}

fn materialize_uisp_route_overrides(overrides: &OverrideFile) -> Vec<RouteOverride> {
    let Some(uisp) = overrides.uisp() else {
        return Vec::new();
    };

    uisp.route_overrides
        .iter()
        .map(|r| RouteOverride {
            from_site: r.from_site.clone(),
            to_site: r.to_site.clone(),
            cost: r.cost,
        })
        .collect()
}

#[allow(dead_code)]
pub fn write_routing_overrides_template(
    config: Arc<Config>,
    natural_routes: &[RouteOverride],
) -> anyhow::Result<()> {
    let file_path = Path::new(&config.lqos_directory).join("integrationUISProutes.template.csv");
    let mut writer = csv::Writer::from_path(file_path)?;
    writer.write_record(["From Site", "To Site", "Cost"])?;
    for route in natural_routes {
        writer.write_record([&route.from_site, &route.to_site, &route.cost.to_string()])?;
    }
    writer.flush()?;
    Ok(())
}
