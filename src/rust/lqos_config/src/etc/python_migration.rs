//! This module utilizes PyO3 to read an existing ispConfig.py file, and
//! provide conversion services for the new, unified configuration target
//! for version 1.5.

use super::EtcLqos;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;
use tracing::error;

#[derive(Debug, Error)]
pub enum PythonMigrationError {
    #[error("The ispConfig.py file does not exist.")]
    ConfigFileNotFound,
    #[error("String not readable UTF-8")]
    BadString,
    #[error("Serialization Error: {e:?}")]
    Serialize { e: Box<dyn std::error::Error> },
}

fn isp_config_py_path(cfg: &EtcLqos) -> PathBuf {
    let base_path = Path::new(&cfg.lqos_directory);
    base_path.join("ispConfig.py")
}

/// Does thie ispConfig.py file exist?
fn config_exists(cfg: &EtcLqos) -> bool {
    isp_config_py_path(cfg).exists()
}

#[derive(Serialize, Deserialize, Default)]
pub struct ExceptionCpes {}

#[derive(Serialize, Deserialize, Default)]
pub struct PythonMigration {
    pub sqm: String,
    #[serde(rename = "monitorOnlyMode")]
    pub monitor_only_mode: bool,
    #[serde(rename = "upstreamBandwidthCapacityDownloadMbps")]
    pub upstream_bandwidth_capacity_download_mbps: i64,
    #[serde(rename = "upstreamBandwidthCapacityUploadMbps")]
    pub upstream_bandwidth_capacity_upload_mbps: i64,
    #[serde(rename = "generatedPNDownloadMbps")]
    pub generated_pndownload_mbps: i64,
    #[serde(rename = "generatedPNUploadMbps")]
    pub generated_pnupload_mbps: i64,
    #[serde(rename = "interfaceA")]
    pub interface_a: String,
    #[serde(rename = "interfaceB")]
    pub interface_b: String,
    #[serde(rename = "queueRefreshIntervalMins")]
    pub queue_refresh_interval_mins: i64,
    #[serde(rename = "OnAStick")]
    pub on_astick: bool,
    #[serde(rename = "StickVlanA")]
    pub stick_vlan_a: i64,
    #[serde(rename = "StickVlanB")]
    pub stick_vlan_b: i64,
    #[serde(rename = "enableActualShellCommands")]
    pub enable_actual_shell_commands: bool,
    #[serde(rename = "runShellCommandsAsSudo")]
    pub run_shell_commands_as_sudo: bool,
    #[serde(rename = "queuesAvailableOverride")]
    pub queues_available_override: i64,
    #[serde(rename = "useBinPackingToBalanceCPU")]
    pub use_bin_packing_to_balance_cpu: bool,
    #[serde(rename = "influxEnabled")]
    pub influx_enabled: bool,
    #[serde(rename = "influxDBurl")]
    pub influx_dburl: String,
    #[serde(rename = "influxDBBucket")]
    pub influx_dbbucket: String,
    #[serde(rename = "influxDBOrg")]
    pub influx_dborg: String,
    #[serde(rename = "influxDBtoken")]
    pub influx_dbtoken: String,
    #[serde(rename = "circuitNameUseAddress")]
    pub circuit_name_use_address: bool,
    #[serde(rename = "overwriteNetworkJSONalways")]
    pub overwrite_network_jsonalways: bool,
    #[serde(rename = "ignoreSubnets")]
    pub ignore_subnets: Vec<String>,
    #[serde(rename = "allowedSubnets")]
    pub allowed_subnets: Vec<String>,
    #[serde(rename = "excludeSites")]
    pub exclude_sites: Vec<String>,
    #[serde(rename = "findIPv6usingMikrotikAPI")]
    pub find_ipv6using_mikrotik_api: bool,
    #[serde(rename = "automaticImportSplynx")]
    pub automatic_import_splynx: bool,
    pub splynx_api_key: String,
    pub splynx_api_secret: String,
    pub splynx_api_url: String,
    #[serde(rename = "automaticImportNetzur")]
    pub automatic_import_netzur: bool,
    pub netzur_api_key: String,
    pub netzur_api_url: String,
    #[serde(rename = "netzur_api_timeout")]
    pub netzur_api_timeout: i64,
    #[serde(rename = "automaticImportUISP")]
    pub automatic_import_uisp: bool,
    #[serde(rename = "uispAuthToken")]
    pub uisp_auth_token: String,
    #[serde(rename = "UISPbaseURL")]
    pub uispbase_url: String,
    #[serde(rename = "uispSite")]
    pub uisp_site: String,
    #[serde(rename = "uispStrategy")]
    pub uisp_strategy: String,
    #[serde(rename = "uispSuspendedStrategy")]
    pub uisp_suspended_strategy: String,
    #[serde(rename = "airMax_capacity")]
    pub air_max_capacity: f64,
    pub ltu_capacity: f64,
    #[serde(rename = "bandwidthOverheadFactor")]
    pub bandwidth_overhead_factor: f64,
    #[serde(rename = "committedBandwidthMultiplier")]
    pub committed_bandwidth_multiplier: f64,
    #[serde(rename = "exceptionCPEs")]
    pub exception_cpes: ExceptionCpes,
    #[serde(rename = "apiUsername")]
    pub api_username: String,
    #[serde(rename = "apiPassword")]
    pub api_password: String,
    #[serde(rename = "apiHostIP")]
    pub api_host_ip: String,
    #[serde(rename = "apiHostPost")]
    pub api_host_post: i64,
    #[serde(rename = "automaticImportPowercode")]
    pub automatic_import_powercode: bool,
    pub powercode_api_key: String,
    pub powercode_api_url: String,
    #[serde(rename = "automaticImportSonar")]
    pub automatic_import_sonar: bool,
    pub sonar_api_key: String,
    pub sonar_api_url: String,
    pub snmp_community: String,
    pub sonar_active_status_ids: Vec<String>,
    pub sonar_airmax_ap_model_ids: Vec<String>,
    pub sonar_ltu_ap_model_ids: Vec<String>,
}

impl PythonMigration {
    pub fn load() -> Result<Self, PythonMigrationError> {
        if let Ok(cfg) = crate::etc::EtcLqos::load() {
            if !config_exists(&cfg) {
                return Err(PythonMigrationError::ConfigFileNotFound);
            }

            let migrator_path = Path::new(&cfg.lqos_directory).join("configMigrator.py");
            if !migrator_path.exists() {
                return Err(PythonMigrationError::ConfigFileNotFound);
            }
            let output = Command::new("/usr/bin/python3")
                .arg(migrator_path)
                .output()
                .map_err(|e| {
                    error!("Error running Python migrator: {:?}", e);
                    PythonMigrationError::ConfigFileNotFound
                })?;
            if !output.status.success() {
                error!("Error running Python migrator: {:?}", output);
                return Err(PythonMigrationError::ConfigFileNotFound);
            }
            let json =
                String::from_utf8(output.stdout).map_err(|_| PythonMigrationError::BadString)?;
            let json: Self = serde_json::from_str(&json)
                .map_err(|e| PythonMigrationError::Serialize { e: Box::new(e) })?;
            Ok(json)
        } else {
            Err(PythonMigrationError::ConfigFileNotFound)
        }
    }
}
