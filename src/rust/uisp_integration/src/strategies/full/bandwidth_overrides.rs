use crate::errors::UispIntegrationError;
use crate::uisp_types::UispSite;
use csv::ReaderBuilder;
use lqos_config::Config;
use lqos_overrides::{NetworkAdjustment, OverrideFile};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};

const LEGACY_UISP_BANDWIDTH_FILE: &str = "integrationUISPbandwidths.csv";

/// One UISP bandwidth override resolved from operator overrides or legacy CSV.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BandwidthOverride {
    /// Stable node identifier, when the override originated from the operator tree UI.
    pub node_id: Option<String>,
    /// Display name of the target site or AP.
    pub site_name: String,
    /// Replacement download bandwidth in Mbps.
    pub download_bandwidth_mbps: Option<f32>,
    /// Replacement upload bandwidth in Mbps.
    pub upload_bandwidth_mbps: Option<f32>,
}

/// Attempts to load UISP bandwidth overrides.
///
/// Current precedence:
/// 1. Operator `AdjustSiteSpeed` overrides from `lqos_overrides.json`
/// 2. One-time migration from `integrationUISPbandwidths.csv` into operator overrides
///
/// Deprecated legacy `uisp.bandwidth_overrides` entries are intentionally ignored.
pub fn get_site_bandwidth_overrides(
    config: &Config,
) -> Result<Vec<BandwidthOverride>, UispIntegrationError> {
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
        && !uisp.bandwidth_overrides.is_empty()
    {
        warn!(
            path = %operator_path.display(),
            count = uisp.bandwidth_overrides.len(),
            "Deprecated legacy uisp.bandwidth_overrides entries are present but ignored. Use operator AdjustSiteSpeed overrides in lqos_overrides.json instead"
        );
    }

    let mut materialized = materialize_operator_site_bandwidth_overrides(&operator_overrides);
    if !materialized.is_empty() {
        let legacy_path = legacy_bandwidth_csv_path(config);
        if legacy_path.exists() {
            warn!(
                "Legacy {} is present but operator rate overrides already exist in lqos_overrides.json; the CSV is ignored.",
                LEGACY_UISP_BANDWIDTH_FILE
            );
        }
        info!(
            "Using {} UISP bandwidth override(s) from operator AdjustSiteSpeed entries in lqos_overrides.json",
            materialized.len()
        );
        return Ok(materialized);
    }

    if let Some(migrated) =
        try_migrate_legacy_csv_to_operator_overrides(config, &mut operator_overrides)?
    {
        materialized = materialize_operator_site_bandwidth_overrides(&operator_overrides);
        if materialized.is_empty() {
            return Ok(migrated);
        }
        return Ok(materialized);
    }

    info!("No UISP bandwidth overrides loaded.");
    Ok(Vec::new())
}

fn load_operator_override_file(operator_path: &Path) -> anyhow::Result<OverrideFile> {
    if !operator_path.exists() {
        return Ok(OverrideFile::default());
    }
    OverrideFile::load_from_explicit_path(operator_path)
}

fn legacy_bandwidth_csv_path(config: &Config) -> PathBuf {
    Path::new(&config.lqos_directory).join(LEGACY_UISP_BANDWIDTH_FILE)
}

fn materialize_operator_site_bandwidth_overrides(
    overrides: &OverrideFile,
) -> Vec<BandwidthOverride> {
    overrides
        .network_adjustments()
        .iter()
        .filter_map(|adjustment| match adjustment {
            NetworkAdjustment::AdjustSiteSpeed {
                node_id,
                site_name,
                download_bandwidth_mbps,
                upload_bandwidth_mbps,
            } => Some(BandwidthOverride {
                node_id: node_id.clone(),
                site_name: site_name.clone(),
                download_bandwidth_mbps: *download_bandwidth_mbps,
                upload_bandwidth_mbps: *upload_bandwidth_mbps,
            }),
            _ => None,
        })
        .collect()
}

fn try_migrate_legacy_csv_to_operator_overrides(
    config: &Config,
    operator_overrides: &mut OverrideFile,
) -> Result<Option<Vec<BandwidthOverride>>, UispIntegrationError> {
    let file_path = legacy_bandwidth_csv_path(config);
    if !file_path.exists() {
        return Ok(None);
    }

    info!("Looking for {LEGACY_UISP_BANDWIDTH_FILE}");
    let migrated = load_bandwidth_overrides_from_csv(&file_path)?;
    if migrated.is_empty() {
        warn!(
            "Legacy {} exists but no usable rows were found; leaving the file untouched.",
            LEGACY_UISP_BANDWIDTH_FILE
        );
        return Ok(Some(Vec::new()));
    }

    for override_row in &migrated {
        operator_overrides.set_site_bandwidth_override(
            None,
            override_row.site_name.clone(),
            override_row.download_bandwidth_mbps,
            override_row.upload_bandwidth_mbps,
        );
    }

    let operator_path = OverrideFile::operator_path_for_config(config);
    if let Err(err) = operator_overrides.save_to_explicit_path(&operator_path) {
        warn!(
            "Unable to save migrated UISP bandwidth overrides to {}: {err:?}. Falling back to the legacy CSV for this run.",
            operator_path.display()
        );
        return Ok(Some(migrated));
    }

    if let Err(err) = rename_legacy_csv_to_backup(&file_path) {
        warn!(
            "Migrated legacy UISP bandwidth overrides into lqos_overrides.json, but could not rename {} to a backup: {err:?}",
            file_path.display()
        );
    } else {
        info!(
            "Migrated legacy {} into operator AdjustSiteSpeed overrides in lqos_overrides.json",
            LEGACY_UISP_BANDWIDTH_FILE
        );
    }

    Ok(Some(migrated))
}

fn load_bandwidth_overrides_from_csv(
    file_path: &Path,
) -> Result<Vec<BandwidthOverride>, UispIntegrationError> {
    let reader = ReaderBuilder::new()
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
    for (line, result) in reader.records().enumerate() {
        if let Ok(result) = result {
            if result.len() != 3 {
                error!(
                    "Wrong number of records in {} on line {line}",
                    file_path.display()
                );
                continue;
            }
            let site_name = result[0].to_string();
            if let Some(down) = numeric_string_to_f32(&result[1]) {
                if let Some(up) = numeric_string_to_f32(&result[2]) {
                    info!(
                        "Loaded legacy UISP bandwidth override from CSV: {}, {}/{}",
                        site_name, down, up
                    );
                    overrides.push(BandwidthOverride {
                        node_id: None,
                        site_name,
                        download_bandwidth_mbps: Some(down),
                        upload_bandwidth_mbps: Some(up),
                    });
                } else {
                    error!("Cannot parse {} as float on line {line}", &result[2]);
                }
            } else {
                error!("Cannot parse {} as float on line {line}", &result[1]);
            }
        } else {
            error!("Error reading {} line", file_path.display());
            error!("{result:?}");
        }
    }

    info!(
        "Loaded {} legacy UISP bandwidth override row(s)",
        overrides.len()
    );
    Ok(overrides)
}

fn rename_legacy_csv_to_backup(file_path: &Path) -> std::io::Result<()> {
    let unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let backup_name = format!("{LEGACY_UISP_BANDWIDTH_FILE}.migrated-{unix_secs}");
    let backup_path = file_path.with_file_name(backup_name);
    fs::rename(file_path, backup_path)
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

/// Finds the bandwidth override that best matches `site_name` and `node_id`.
///
/// Preference order:
/// 1. Exact `node_id` match when one is supplied.
/// 2. Legacy name-only match for the same `site_name`.
pub fn find_bandwidth_override<'a>(
    overrides: &'a [BandwidthOverride],
    node_id: Option<&str>,
    site_name: &str,
) -> Option<&'a BandwidthOverride> {
    if let Some(node_id) = node_id
        && let Some(found) = overrides
            .iter()
            .find(|entry| entry.node_id.as_deref() == Some(node_id))
    {
        return Some(found);
    }

    overrides
        .iter()
        .find(|entry| entry.node_id.is_none() && entry.site_name == site_name)
}

/// Applies bandwidth overrides to the UISP site list.
pub fn apply_bandwidth_overrides(
    sites: &mut [UispSite],
    bandwidth_overrides: &[BandwidthOverride],
) {
    for site in sites.iter_mut() {
        let node_id = format!("uisp:site:{}", site.id);
        if let Some(bandwidth_override) =
            find_bandwidth_override(bandwidth_overrides, Some(&node_id), &site.name)
        {
            if let Some(down) = bandwidth_override.download_bandwidth_mbps {
                site.max_down_mbps = down as u64;
            }
            if let Some(up) = bandwidth_override.upload_bandwidth_mbps {
                site.max_up_mbps = up as u64;
            }
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
    use lqos_config::Config;
    use lqos_overrides::NetworkAdjustment;

    fn unique_temp_dir(name: &str) -> PathBuf {
        let unique = format!(
            "libreqos-uisp-bw-{}-{}-{}",
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

    #[test]
    fn operator_adjust_site_speed_overrides_take_precedence() {
        let dir = unique_temp_dir("operator-precedence");
        let config = config_for_dir(&dir);
        let operator_path = OverrideFile::operator_path_for_config(&config);

        let mut overrides = OverrideFile::default();
        overrides.set_site_bandwidth_override(
            Some("uisp:site:site-1".to_string()),
            "Site1".to_string(),
            Some(123.0),
            Some(45.0),
        );
        overrides
            .save_to_explicit_path(&operator_path)
            .expect("operator overrides should save");
        fs::write(
            legacy_bandwidth_csv_path(&config),
            "Parent Node,Down,Up\nSite1,100,10\n",
        )
        .expect("legacy csv should write");

        let loaded = get_site_bandwidth_overrides(&config).expect("overrides should load");
        assert_eq!(
            loaded,
            vec![BandwidthOverride {
                node_id: Some("uisp:site:site-1".to_string()),
                site_name: "Site1".to_string(),
                download_bandwidth_mbps: Some(123.0),
                upload_bandwidth_mbps: Some(45.0),
            }]
        );
        assert!(legacy_bandwidth_csv_path(&config).exists());
    }

    #[test]
    fn legacy_uisp_json_bandwidth_overrides_are_ignored() {
        let dir = unique_temp_dir("legacy-json-ignored");
        let config = config_for_dir(&dir);
        let operator_path = OverrideFile::operator_path_for_config(&config);

        let mut overrides = OverrideFile::default();
        overrides.set_uisp_bandwidth_override("Site1".to_string(), 222.0, 111.0);
        overrides
            .save_to_explicit_path(&operator_path)
            .expect("operator overrides should save");

        let loaded = get_site_bandwidth_overrides(&config).expect("overrides should load");
        assert!(loaded.is_empty());
    }

    #[test]
    fn legacy_csv_is_migrated_into_adjust_site_speed_overrides() {
        let dir = unique_temp_dir("csv-migrate");
        let config = config_for_dir(&dir);
        fs::write(
            legacy_bandwidth_csv_path(&config),
            "Parent Node,Down,Up\nSite1,100,10\n",
        )
        .expect("legacy csv should write");

        let loaded = get_site_bandwidth_overrides(&config).expect("overrides should load");
        assert_eq!(
            loaded,
            vec![BandwidthOverride {
                node_id: None,
                site_name: "Site1".to_string(),
                download_bandwidth_mbps: Some(100.0),
                upload_bandwidth_mbps: Some(10.0),
            }]
        );
        assert!(!legacy_bandwidth_csv_path(&config).exists());

        let operator_path = OverrideFile::operator_path_for_config(&config);
        let saved = OverrideFile::load_from_explicit_path(&operator_path)
            .expect("saved overrides should load");
        assert!(matches!(
            saved.find_site_bandwidth_override(None, "Site1"),
            Some(NetworkAdjustment::AdjustSiteSpeed {
                download_bandwidth_mbps: Some(100.0),
                upload_bandwidth_mbps: Some(10.0),
                ..
            })
        ));
    }

    #[test]
    fn partial_operator_overrides_are_applied_per_direction() {
        let mut site = UispSite {
            id: "site-1".to_string(),
            name: "Site1".to_string(),
            max_down_mbps: 200,
            max_up_mbps: 50,
            ..UispSite::default()
        };
        let overrides = vec![BandwidthOverride {
            node_id: Some("uisp:site:site-1".to_string()),
            site_name: "Site1".to_string(),
            download_bandwidth_mbps: Some(150.0),
            upload_bandwidth_mbps: None,
        }];

        apply_bandwidth_overrides(std::slice::from_mut(&mut site), &overrides);
        assert_eq!(site.max_down_mbps, 150);
        assert_eq!(site.max_up_mbps, 50);
    }
}
