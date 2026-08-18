//! Determines whether runtime topology onboarding is still required after
//! `lqos_setup` has handed control to `lqosd`.

use lqos_config::{Config, load_config};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::node_manager::local_api::config::active_topology_source_integrations;

/// Describes the topology source family currently detected by `lqosd`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RuntimeTopologySourceKind {
    #[default]
    None,
    Integration,
    ManualFiles,
}

/// Reports whether the operator still needs to finish runtime topology setup.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct RuntimeOnboardingState {
    /// True when the UI should keep steering the operator into topology setup.
    pub required: bool,
    /// Short operator-facing state label.
    pub status_label: String,
    /// Short guidance text explaining the next required action.
    pub summary: String,
    /// Topology source family currently present.
    pub source_kind: RuntimeTopologySourceKind,
    /// Enabled integration names that currently own topology input.
    pub active_integrations: Vec<String>,
    /// Whether `network.json` exists in the configured LibreQoS directory.
    pub network_json_present: bool,
    /// Whether `ShapedDevices.csv` exists in the configured LibreQoS directory.
    pub shaped_devices_present: bool,
    /// Alert severity for the onboarding status banner.
    pub status_severity: String,
}

fn topology_files_present(config: &Config) -> (bool, bool) {
    let base = Path::new(&config.lqos_directory);
    (
        base.join("network.json").exists(),
        base.join("ShapedDevices.csv").exists(),
    )
}

fn from_config(config: &Config) -> RuntimeOnboardingState {
    let active_integrations = active_topology_source_integrations(config)
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let (network_json_present, shaped_devices_present) = topology_files_present(config);

    if !active_integrations.is_empty() {
        return RuntimeOnboardingState {
            required: false,
            status_label: "Configured".to_string(),
            summary: format!(
                "Topology source is configured through {}. Topology data may still be publishing.",
                active_integrations.join(", ")
            ),
            source_kind: RuntimeTopologySourceKind::Integration,
            active_integrations,
            network_json_present,
            shaped_devices_present,
            status_severity: "info".to_string(),
        };
    }

    if network_json_present && shaped_devices_present {
        return RuntimeOnboardingState {
            required: false,
            status_label: "Ready".to_string(),
            summary: "Manual topology files are present.".to_string(),
            source_kind: RuntimeTopologySourceKind::ManualFiles,
            active_integrations,
            network_json_present,
            shaped_devices_present,
            status_severity: "success".to_string(),
        };
    }

    let source_kind = if network_json_present || shaped_devices_present {
        RuntimeTopologySourceKind::ManualFiles
    } else {
        RuntimeTopologySourceKind::None
    };
    let summary = match (network_json_present, shaped_devices_present) {
        (false, false) => {
            "Choose a topology source before expecting scheduler activity.".to_string()
        }
        (false, true) => "ShapedDevices.csv exists, but network.json is still missing.".to_string(),
        (true, false) => "network.json exists, but ShapedDevices.csv is still missing.".to_string(),
        (true, true) => unreachable!("handled above"),
    };

    RuntimeOnboardingState {
        required: true,
        status_label: "Setup Required".to_string(),
        summary,
        source_kind,
        active_integrations,
        network_json_present,
        shaped_devices_present,
        status_severity: "warning".to_string(),
    }
}

fn config_error_state() -> RuntimeOnboardingState {
    RuntimeOnboardingState {
        required: true,
        status_label: "Config Error".to_string(),
        summary: "Unable to load LibreQoS configuration. Resolve the config error before continuing runtime setup.".to_string(),
        source_kind: RuntimeTopologySourceKind::None,
        active_integrations: Vec::new(),
        network_json_present: false,
        shaped_devices_present: false,
        status_severity: "danger".to_string(),
    }
}

/// Loads the current runtime onboarding state from config and topology files.
///
/// On config-read failure this returns a blocking error state so the UI does not
/// present an internal problem as a successful topology setup.
pub(crate) fn runtime_onboarding_state() -> RuntimeOnboardingState {
    match load_config() {
        Ok(config) => from_config(config.as_ref()),
        Err(_) => config_error_state(),
    }
}

#[cfg(test)]
mod tests {
    use super::{RuntimeTopologySourceKind, config_error_state, from_config};
    use lqos_config::Config;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "libreqos-runtime-onboarding-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn requires_setup_when_no_integrations_or_manual_files_exist() {
        let dir = test_dir("missing");
        let config = Config {
            lqos_directory: dir.to_string_lossy().into_owned(),
            ..Config::default()
        };

        let state = from_config(&config);

        assert!(state.required);
        assert_eq!(state.source_kind, RuntimeTopologySourceKind::None);
        assert!(!state.network_json_present);
        assert!(!state.shaped_devices_present);

        fs::remove_dir_all(dir).expect("remove temp dir");
    }

    #[test]
    fn manual_files_complete_runtime_onboarding() {
        let dir = test_dir("manual");
        fs::write(dir.join("network.json"), "{}\n").expect("write network json");
        fs::write(dir.join("ShapedDevices.csv"), "Circuit ID,Device ID\n")
            .expect("write shaped devices");

        let config = Config {
            lqos_directory: dir.to_string_lossy().into_owned(),
            ..Config::default()
        };

        let state = from_config(&config);

        assert!(!state.required);
        assert_eq!(state.source_kind, RuntimeTopologySourceKind::ManualFiles);
        assert!(state.network_json_present);
        assert!(state.shaped_devices_present);

        fs::remove_dir_all(dir).expect("remove temp dir");
    }

    #[test]
    fn enabled_integration_is_configured_but_not_marked_ready() {
        let dir = test_dir("integration");
        let mut config = Config {
            lqos_directory: dir.to_string_lossy().into_owned(),
            ..Config::default()
        };
        config.splynx_integration.enable_splynx = true;

        let state = from_config(&config);

        assert!(!state.required);
        assert_eq!(state.source_kind, RuntimeTopologySourceKind::Integration);
        assert_eq!(state.active_integrations, vec!["Splynx".to_string()]);
        assert_eq!(state.status_label, "Configured");
        assert_eq!(state.status_severity, "info");

        fs::remove_dir_all(dir).expect("remove temp dir");
    }

    #[test]
    fn config_error_state_blocks_runtime_onboarding() {
        let state = config_error_state();
        assert!(state.required);
        assert_eq!(state.status_label, "Config Error");
        assert_eq!(state.status_severity, "danger");
    }
}
