//! This module utilizes PyO3 to read an existing ispConfig.py file, and
//! provide conversion services for the new, unified configuration target
//! for version 1.5.

use crate::EtcLqos;
use pyo3::{prepare_freethreaded_python, Python};
use std::{
    fs::read_to_string,
    path::{Path, PathBuf}, collections::HashMap,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PythonMigrationError {
    #[error("The ispConfig.py file does not exist.")]
    ConfigFileNotFound,
    #[error("Unable to parse variable")]
    ParseError,
    #[error("Variable not found")]
    VariableNotFound(String),
}

fn isp_config_py_path(cfg: &EtcLqos) -> PathBuf {
    let base_path = Path::new(&cfg.lqos_directory);
    let final_path = base_path.join("ispConfig.py");
    final_path
}

/// Does thie ispConfig.py file exist?
fn config_exists(cfg: &EtcLqos) -> bool {
    if let Ok(cfg) = crate::etc::EtcLqos::load() {
        isp_config_py_path(&cfg).exists()
    } else {
        false
    }
}

fn from_python<'a, T>(py: &'a Python, variable_name: &str) -> Result<T, PythonMigrationError>
where
    T: pyo3::FromPyObject<'a>,
{
    let result = py
        .eval(variable_name, None, None)
        .map_err(|_| PythonMigrationError::VariableNotFound(variable_name.to_string()))?
        .extract::<T>()
        .map_err(|_| PythonMigrationError::ParseError)?;

    Ok(result)
}

#[derive(Default, Debug)]
pub struct PythonMigration {
    pub sqm: String,
    pub monitor_only_mode: bool,
    pub upstream_bandwidth_capacity_download_mbps: u32,
    pub upstream_bandwidth_capacity_upload_mbps: u32,
    pub generated_pn_download_mbps: u32,
    pub generated_pn_upload_mbps: u32,
    pub interface_a: String,
    pub interface_b: String,
    pub queue_refresh_interval_mins: u32,
    pub on_a_stick: bool,
    pub stick_vlan_a: u32,
    pub stick_vlan_b: u32,
    pub enable_actual_shell_commands: bool,
    pub run_shell_commands_as_sudo: bool,
    pub queues_available_override: u32,
    pub use_bin_packing_to_balance_cpu: bool,
    pub influx_db_enabled: bool,
    pub influx_db_url: String,
    pub infux_db_bucket: String,
    pub influx_db_org: String,
    pub influx_db_token: String,
    pub circuit_name_use_address: bool,
    pub overwrite_network_json_always: bool,
    pub ignore_subnets: Vec<String>,
    pub allowed_subnets: Vec<String>,
    pub automatic_import_splynx: bool,
    pub splynx_api_key: String,
    pub spylnx_api_secret: String,
    pub spylnx_api_url: String,
    pub automatic_import_uisp: bool,
    pub uisp_auth_token: String,
    pub uisp_base_url: String,
    pub uisp_site: String,
    pub uisp_strategy: String,
    pub uisp_suspended_strategy: String,
    pub airmax_capacity: f32,
    pub ltu_capacity: f32,
    pub exclude_sites: Vec<String>,
    pub find_ipv6_using_mikrotik: bool,
    pub bandwidth_overhead_factor: f32,
    pub committed_bandwidth_multiplier: f32,
    pub exception_cpes: HashMap<String, String>,
    pub api_username: String,
    pub api_password: String,
    pub api_host_ip: String,
    pub api_host_port: u32,

    // TODO: httpRestIntegrationConfig
}

impl PythonMigration {
    fn parse(cfg: &mut Self, py: &Python) -> Result<(), PythonMigrationError> {
        cfg.sqm = from_python(&py, "sqm")?;
        cfg.monitor_only_mode = from_python(&py, "monitorOnlyMode")?;
        cfg.upstream_bandwidth_capacity_download_mbps =
            from_python(&py, "upstreamBandwidthCapacityDownloadMbps")?;
        cfg.upstream_bandwidth_capacity_upload_mbps =
            from_python(&py, "upstreamBandwidthCapacityUploadMbps")?;
        cfg.generated_pn_download_mbps = from_python(&py, "generatedPNDownloadMbps")?;
        cfg.generated_pn_upload_mbps = from_python(&py, "generatedPNUploadMbps")?;
        cfg.interface_a = from_python(&py, "interfaceA")?;
        cfg.interface_b = from_python(&py, "interfaceB")?;
        cfg.queue_refresh_interval_mins = from_python(&py, "queueRefreshIntervalMins")?;
        cfg.on_a_stick = from_python(&py, "OnAStick")?;
        cfg.stick_vlan_a = from_python(&py, "StickVlanA")?;
        cfg.stick_vlan_b = from_python(&py, "StickVlanB")?;
        cfg.enable_actual_shell_commands = from_python(&py, "enableActualShellCommands")?;
        cfg.run_shell_commands_as_sudo = from_python(&py, "runShellCommandsAsSudo")?;
        cfg.queues_available_override = from_python(&py, "queuesAvailableOverride")?;
        cfg.use_bin_packing_to_balance_cpu = from_python(&py, "useBinPackingToBalanceCPU")?;
        cfg.influx_db_enabled = from_python(&py, "influxDBEnabled")?;
        cfg.influx_db_url = from_python(&py, "influxDBurl")?;
        cfg.infux_db_bucket = from_python(&py, "influxDBBucket")?;
        cfg.influx_db_org = from_python(&py, "influxDBOrg")?;
        cfg.influx_db_token = from_python(&py, "influxDBtoken")?;
        cfg.circuit_name_use_address = from_python(&py, "circuitNameUseAddress")?;
        cfg.overwrite_network_json_always = from_python(&py, "overwriteNetworkJSONalways")?;
        cfg.ignore_subnets = from_python(&py, "ignoreSubnets")?;
        cfg.allowed_subnets = from_python(&py, "allowedSubnets")?;
        cfg.automatic_import_splynx = from_python(&py, "automaticImportSplynx")?;
        cfg.splynx_api_key = from_python(&py, "splynx_api_key")?;
        cfg.spylnx_api_secret = from_python(&py, "splynx_api_secret")?;
        cfg.spylnx_api_url = from_python(&py, "splynx_api_url")?;
        cfg.automatic_import_uisp = from_python(&py, "automaticImportUISP")?;
        cfg.uisp_auth_token = from_python(&py, "uispAuthToken")?;
        cfg.uisp_base_url = from_python(&py, "UISPbaseURL")?;
        cfg.uisp_site = from_python(&py, "uispSite")?;
        cfg.uisp_strategy = from_python(&py, "uispStrategy")?;
        cfg.uisp_suspended_strategy = from_python(&py, "uispSuspendedStrategy")?;
        cfg.airmax_capacity = from_python(&py, "airMax_capacity")?;
        cfg.ltu_capacity = from_python(&py, "ltu_capacity")?;
        cfg.exclude_sites = from_python(&py, "excludeSites")?;
        cfg.find_ipv6_using_mikrotik = from_python(&py, "findIPv6usingMikrotik")?;
        cfg.bandwidth_overhead_factor = from_python(&py, "bandwidthOverheadFactor")?;
        cfg.committed_bandwidth_multiplier = from_python(&py, "committedBandwidthMultiplier")?;
        cfg.exception_cpes = from_python(&py, "exceptionCPEs")?;
        cfg.api_username = from_python(&py, "apiUsername")?;
        cfg.api_password = from_python(&py, "apiPassword")?;
        cfg.api_host_ip = from_python(&py, "apiHostIP")?;
        cfg.api_host_port = from_python(&py, "apiHostPost")?;

        Ok(())
    }

    pub fn load() -> Result<Self, PythonMigrationError> {
        let mut old_config = Self::default();
        if let Ok(cfg) = crate::etc::EtcLqos::load() {
            if !config_exists(&cfg) {
                return Err(PythonMigrationError::ConfigFileNotFound);
            }
            let code = read_to_string(isp_config_py_path(&cfg)).unwrap();

            prepare_freethreaded_python();
            Python::with_gil(|py| {
                py.run(&code, None, None).unwrap();
                let result = Self::parse(&mut old_config, &py);
                if result.is_err() {
                    println!("Error parsing Python config: {:?}", result);
                }
            });
        } else {
            return Err(PythonMigrationError::ConfigFileNotFound);
        }

        Ok(old_config)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    const DEFAULT_ISP_CONFIG_PY: &str = include_str!("../../../../ispConfig.example.py");

    #[test]
    fn test_parsing_the_default() {
        let mut cfg = PythonMigration::default();
        prepare_freethreaded_python();
        let mut worked = true;
        Python::with_gil(|py| {
            py.run(DEFAULT_ISP_CONFIG_PY, None, None).unwrap();
            let result = PythonMigration::parse(&mut cfg, &py);
            if result.is_err() {
                println!("Error parsing Python config: {:?}", result);
                worked = false;
            }
        });
        assert!(worked)
    }
}