//! Determines whether runtime topology onboarding is still required after
//! `lqos_setup` has handed control to `lqosd`.

use lqos_config::{Config, TopologySource, load_config};
use serde::{Deserialize, Serialize};
use std::path::Path;


/// Describes the topology source family currently detected by `lqosd`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RuntimeTopologySourceKind {
    #[default]
    None,
    Integration,
    ManualFiles,
    LibreqosNetwork,
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

fn source_kind(source: Option<TopologySource>) -> RuntimeTopologySourceKind {
    match source {
        Some(source) if source.is_integration() => RuntimeTopologySourceKind::Integration,
        Some(TopologySource::ShapedDevices) => RuntimeTopologySourceKind::ManualFiles,
        Some(TopologySource::LibreqosNetwork) => RuntimeTopologySourceKind::LibreqosNetwork,
        Some(_) => RuntimeTopologySourceKind::None,
        None => RuntimeTopologySourceKind::None,
    }
}

fn from_config(config: &Config) -> RuntimeOnboardingState {
    let source = config.topology.source;
    let active_integrations = source
        .filter(|source| source.is_integration())
        .map(|source| vec![source.display_name().to_string()])
        .unwrap_or_default();
    let (network_json_present, shaped_devices_present) = topology_files_present(config);

    if let Some(conflict) = config.topology_source_runtime.conflict.as_ref() {
        return RuntimeOnboardingState {
            required: true,
            status_label: "Source Conflict".to_string(),
            summary: conflict.message.clone(),
            source_kind: source_kind(source),
            active_integrations,
            network_json_present,
            shaped_devices_present,
            status_severity: "danger".to_string(),
        };
    }

    if let Some(source) = source {
        match source {
            TopologySource::ShapedDevices => {
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
                let summary = match (network_json_present, shaped_devices_present) {
                    (false, false) => {
                        "Choose a topology source before expecting scheduler activity.".to_string()
                    }
                    (false, true) => {
                        "ShapedDevices.csv exists, but network.json is still missing.".to_string()
                    }
                    (true, false) => {
                        "network.json exists, but ShapedDevices.csv is still missing.".to_string()
                    }
                    (true, true) => unreachable!("handled by the ready branch"),
                };
                RuntimeOnboardingState {
                    required: true,
                    status_label: "Setup Required".to_string(),
                    summary,
                    source_kind: RuntimeTopologySourceKind::ManualFiles,
                    active_integrations,
                    network_json_present,
                    shaped_devices_present,
                    status_severity: "warning".to_string(),
                }
            }
            TopologySource::LibreqosNetwork => RuntimeOnboardingState {
                required: false,
                status_label: "Configured".to_string(),
                summary: "Topology source is the libreqos_network.json drop.".to_string(),
                source_kind: RuntimeTopologySourceKind::LibreqosNetwork,
                active_integrations,
                network_json_present,
                shaped_devices_present,
                status_severity: "info".to_string(),
            },
            _ => RuntimeOnboardingState {
                required: false,
                status_label: "Configured".to_string(),
                summary: format!(
                    "Topology source is configured through {}. Topology data may still be publishing.",
                    source.display_name()
                ),
                source_kind: RuntimeTopologySourceKind::Integration,
                active_integrations,
                network_json_present,
                shaped_devices_present,
                status_severity: "info".to_string(),
            },
        }
    } else {
        RuntimeOnboardingState {
            required: true,
            status_label: "Setup Required".to_string(),
            summary: "Choose a topology source before expecting scheduler activity.".to_string(),
            source_kind: RuntimeTopologySourceKind::None,
            active_integrations,
            network_json_present,
            shaped_devices_present,
            status_severity: "warning".to_string(),
        }
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

        let mut config = Config {
            lqos_directory: dir.to_string_lossy().into_owned(),
            ..Config::default()
        };
        config.topology.source = Some(lqos_config::TopologySource::ShapedDevices);

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
        config.topology.source = Some(lqos_config::TopologySource::Splynx);

        let state = from_config(&config);

        assert!(!state.required);
        assert_eq!(state.source_kind, RuntimeTopologySourceKind::Integration);
        assert_eq!(state.active_integrations, vec!["Splynx".to_string()]);
        assert_eq!(state.status_label, "Configured");
        assert_eq!(state.status_severity, "info");

        fs::remove_dir_all(dir).expect("remove temp dir");
    }

    #[test]
    fn libreqos_network_is_configured_without_a_scheduler_integration() {
        let dir = test_dir("libreqos");
        let mut config = Config {
            lqos_directory: dir.to_string_lossy().into_owned(),
            ..Config::default()
        };
        config.topology.source = Some(lqos_config::TopologySource::LibreqosNetwork);

        let state = from_config(&config);

        assert!(!state.required);
        assert_eq!(state.source_kind, RuntimeTopologySourceKind::LibreqosNetwork);
        assert!(state.active_integrations.is_empty());
        assert_eq!(state.status_label, "Configured");

        fs::remove_dir_all(dir).expect("remove temp dir");
    }

    #[test]
    fn source_conflict_blocks_onboarding_and_names_the_conflict() {
        let dir = test_dir("conflict");
        let mut config = Config {
            lqos_directory: dir.to_string_lossy().into_owned(),
            ..Config::default()
        };
        config.topology.source = Some(lqos_config::TopologySource::Visp);
        config.topology_source_runtime.conflict = Some(
            lqos_config::TopologySourceConflict {
                source: Some(lqos_config::TopologySource::Visp),
                enabled: vec!["enable_uisp".to_string()],
                message: "topology source conflict: enable_uisp is enabled".to_string(),
            },
        );

        let state = from_config(&config);

        assert!(state.required);
        assert_eq!(state.status_label, "Source Conflict");
        assert!(state.summary.contains("enable_uisp"));

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
