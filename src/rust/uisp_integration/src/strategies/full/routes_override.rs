use crate::errors::UispIntegrationError;
use csv::ReaderBuilder;
use lqos_config::Config;
use lqos_overrides::OverrideFile;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};

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
/// Current precedence:
/// 1. `uisp.route_overrides` from `lqos_overrides.json`
/// 2. One-time migration from `integrationUISProutes.csv` into `lqos_overrides.json`
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
    let operator_path = OverrideFile::operator_path_for_config(config);
    let mut operator_overrides = load_operator_override_file(&operator_path).map_err(|err| {
        error!(
            "Unable to load operator overrides from {}",
            operator_path.display()
        );
        error!("{err:?}");
        UispIntegrationError::CsvError
    })?;

    if let Some(uisp) = operator_overrides.uisp()
        && !uisp.route_overrides.is_empty()
    {
        info!("Using UISP route overrides from lqos_overrides.json");
        let legacy_path = legacy_routes_csv_path(config);
        if legacy_path.exists() {
            warn!(
                "Legacy {} is present but UISP route overrides already exist in lqos_overrides.json; the CSV is ignored.",
                LEGACY_UISP_ROUTES_FILE
            );
        }
        return Ok(materialize_uisp_route_overrides(&operator_overrides));
    }

    if let Some(migrated) =
        try_migrate_legacy_csv_to_operator_overrides(config, &mut operator_overrides)?
    {
        let materialized = materialize_uisp_route_overrides(&operator_overrides);
        if materialized.is_empty() {
            return Ok(migrated);
        }
        return Ok(materialized);
    }

    info!("No UISP route overrides loaded.");
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

fn try_migrate_legacy_csv_to_operator_overrides(
    config: &Config,
    operator_overrides: &mut OverrideFile,
) -> Result<Option<Vec<RouteOverride>>, UispIntegrationError> {
    let file_path = legacy_routes_csv_path(config);
    if !file_path.exists() {
        return Ok(None);
    }

    info!("Looking for {LEGACY_UISP_ROUTES_FILE}");
    let migrated = load_route_overrides_from_csv(&file_path)?;
    if migrated.is_empty() {
        warn!(
            "Legacy {} exists but no usable rows were found; leaving the file untouched.",
            LEGACY_UISP_ROUTES_FILE
        );
        return Ok(Some(Vec::new()));
    }

    for route in &migrated {
        operator_overrides.add_uisp_route_override(
            route.from_site.clone(),
            route.to_site.clone(),
            route.cost,
        );
    }

    let operator_path = OverrideFile::operator_path_for_config(config);
    if let Err(err) = operator_overrides.save_to_explicit_path(&operator_path) {
        warn!(
            "Unable to save migrated UISP route overrides to {}: {err:?}. Falling back to the legacy CSV for this run.",
            operator_path.display()
        );
        return Ok(Some(migrated));
    }

    if let Err(err) = rename_legacy_csv_to_backup(&file_path) {
        warn!(
            "Migrated legacy UISP route overrides into lqos_overrides.json, but could not rename {} to a backup: {err:?}",
            file_path.display()
        );
    } else {
        info!(
            "Migrated legacy {} into UISP route overrides in lqos_overrides.json",
            LEGACY_UISP_ROUTES_FILE
        );
    }

    Ok(Some(migrated))
}

fn load_route_overrides_from_csv(
    file_path: &Path,
) -> Result<Vec<RouteOverride>, UispIntegrationError> {
    let reader = ReaderBuilder::new()
        .has_headers(false)
        .comment(Some(b'#'))
        .trim(csv::Trim::All)
        .from_path(file_path);
    let mut reader = match reader {
        Ok(reader) => reader,
        Err(err) => {
            error!("Unable to read {}", file_path.display());
            error!("{err:?}");
            return Err(UispIntegrationError::CsvError);
        }
    };

    let mut overrides = Vec::new();
    for (line_num, rec) in reader.records().enumerate() {
        if let Ok(line) = rec {
            if line.len() != 3 {
                error!(
                    "Wrong number of records in {} on line {}",
                    file_path.display(),
                    line_num
                );
                continue;
            }

            if let Ok(cost) = line[2].parse::<u32>() {
                overrides.push(RouteOverride {
                    from_site: line[0].to_string(),
                    to_site: line[1].to_string(),
                    cost,
                });
            } else {
                error!(
                    "{} is not a valid integer for cost on line {}",
                    &line[2], line_num
                );
            }
        } else {
            error!(
                "Unable to read route overrides CSV line from {}",
                file_path.display()
            );
            error!("{rec:?}");
        }
    }

    info!(
        "Loaded {} legacy UISP route override row(s)",
        overrides.len()
    );
    Ok(overrides)
}

fn rename_legacy_csv_to_backup(file_path: &Path) -> std::io::Result<()> {
    let unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let backup_name = format!("{LEGACY_UISP_ROUTES_FILE}.migrated-{unix_secs}");
    let backup_path = file_path.with_file_name(backup_name);
    fs::rename(file_path, backup_path)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_temp_dir(name: &str) -> PathBuf {
        let unique = format!(
            "libreqos-uisp-route-{}-{}-{}",
            name,
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        fs::create_dir_all(&path).expect("temp dir should be created");
        path
    }

    fn config_for_dir(dir: &Path) -> Config {
        Config {
            lqos_directory: dir.to_string_lossy().into_owned(),
            ..Config::default()
        }
    }

    #[test]
    fn operator_route_overrides_take_precedence() {
        let dir = unique_temp_dir("operator-precedence");
        let config = config_for_dir(&dir);
        let operator_path = OverrideFile::operator_path_for_config(&config);

        let mut overrides = OverrideFile::default();
        overrides.add_uisp_route_override("A".to_string(), "B".to_string(), 42);
        overrides
            .save_to_explicit_path(&operator_path)
            .expect("operator overrides should save");
        fs::write(legacy_routes_csv_path(&config), "A,B,10\n").expect("legacy csv should write");

        let loaded = get_route_overrides(&config).expect("overrides should load");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].from_site, "A");
        assert_eq!(loaded[0].to_site, "B");
        assert_eq!(loaded[0].cost, 42);
        assert!(legacy_routes_csv_path(&config).exists());
    }

    #[test]
    fn legacy_route_csv_is_migrated_into_json_overrides() {
        let dir = unique_temp_dir("csv-migrate");
        let config = config_for_dir(&dir);
        fs::write(legacy_routes_csv_path(&config), "A,B,10\nB,C,25\n")
            .expect("legacy csv should write");

        let loaded = get_route_overrides(&config).expect("overrides should load");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].from_site, "A");
        assert_eq!(loaded[0].to_site, "B");
        assert_eq!(loaded[0].cost, 10);
        assert_eq!(loaded[1].from_site, "B");
        assert_eq!(loaded[1].to_site, "C");
        assert_eq!(loaded[1].cost, 25);
        assert!(!legacy_routes_csv_path(&config).exists());

        let operator_path = OverrideFile::operator_path_for_config(&config);
        let saved = OverrideFile::load_from_explicit_path(&operator_path)
            .expect("saved overrides should load");
        let materialized = materialize_uisp_route_overrides(&saved);
        assert_eq!(materialized.len(), 2);
        assert_eq!(materialized[0].from_site, "A");
        assert_eq!(materialized[0].to_site, "B");
        assert_eq!(materialized[0].cost, 10);
        assert_eq!(materialized[1].from_site, "B");
        assert_eq!(materialized[1].to_site, "C");
        assert_eq!(materialized[1].cost, 25);
    }
}
