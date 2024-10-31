//! This module utilizes PyO3 to read an existing ispConfig.py file, and
//! provide conversion services for the new, unified configuration target
//! for version 1.5.

use super::EtcLqos;
use pyo3::{prepare_freethreaded_python, Python};
use std::{
    collections::HashMap,
    fs::read_to_string,
    path::{Path, PathBuf},
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
    isp_config_py_path(&cfg).exists()
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
    pub automatic_import_powercode: bool,
    pub powercode_api_key: String,
    pub powercode_api_url: String,
    pub automatic_import_sonar: bool,
    pub sonar_api_url: String,
    pub sonar_api_key: String,
    pub snmp_community: String,
    pub sonar_airmax_ap_model_ids: Vec<String>,
    pub sonar_ltu_ap_model_ids: Vec<String>,
    pub sonar_active_status_ids: Vec<String>,

    // TODO: httpRestIntegrationConfig
}

impl PythonMigration {
    fn parse(cfg: &mut Self, py: &Python) -> Result<(), PythonMigrationError> {
        cfg.sqm = from_python(&py, "sqm").unwrap_or("cake diffserv4".to_string());
        cfg.monitor_only_mode = from_python(&py, "monitorOnlyMode").unwrap_or(false);
        cfg.upstream_bandwidth_capacity_download_mbps =
            from_python(&py, "upstreamBandwidthCapacityDownloadMbps").unwrap_or(1000);
        cfg.upstream_bandwidth_capacity_upload_mbps =
            from_python(&py, "upstreamBandwidthCapacityUploadMbps").unwrap_or(1000);
        cfg.generated_pn_download_mbps = from_python(&py, "generatedPNDownloadMbps").unwrap_or(1000);
        cfg.generated_pn_upload_mbps = from_python(&py, "generatedPNUploadMbps").unwrap_or(1000);
        cfg.interface_a = from_python(&py, "interfaceA").unwrap_or("eth1".to_string());
        cfg.interface_b = from_python(&py, "interfaceB").unwrap_or("eth2".to_string());
        cfg.queue_refresh_interval_mins = from_python(&py, "queueRefreshIntervalMins").unwrap_or(15);
        cfg.on_a_stick = from_python(&py, "OnAStick").unwrap_or(false);
        cfg.stick_vlan_a = from_python(&py, "StickVlanA").unwrap_or(0);
        cfg.stick_vlan_b = from_python(&py, "StickVlanB").unwrap_or(0);
        cfg.enable_actual_shell_commands = from_python(&py, "enableActualShellCommands").unwrap_or(true);
        cfg.run_shell_commands_as_sudo = from_python(&py, "runShellCommandsAsSudo").unwrap_or(false);
        cfg.queues_available_override = from_python(&py, "queuesAvailableOverride").unwrap_or(0);
        cfg.use_bin_packing_to_balance_cpu = from_python(&py, "useBinPackingToBalanceCPU").unwrap_or(false);
        
        // Influx
        cfg.influx_db_enabled = from_python(&py, "influxDBEnabled").unwrap_or(false);
        cfg.influx_db_url = from_python(&py, "influxDBurl").unwrap_or("http://localhost:8086".to_string());
        cfg.infux_db_bucket = from_python(&py, "influxDBBucket").unwrap_or("libreqos".to_string());
        cfg.influx_db_org = from_python(&py, "influxDBOrg").unwrap_or("Your ISP Name Here".to_string());
        cfg.influx_db_token = from_python(&py, "influxDBtoken").unwrap_or("".to_string());
        
        // Common
        cfg.circuit_name_use_address = from_python(&py, "circuitNameUseAddress").unwrap_or(true);
        cfg.overwrite_network_json_always = from_python(&py, "overwriteNetworkJSONalways").unwrap_or(false);
        cfg.ignore_subnets = from_python(&py, "ignoreSubnets").unwrap_or(vec!["192.168.0.0/16".to_string()]);
        cfg.allowed_subnets = from_python(&py, "allowedSubnets").unwrap_or(vec!["100.64.0.0/10".to_string()]);
        cfg.exclude_sites = from_python(&py, "excludeSites").unwrap_or(vec![]);
        cfg.find_ipv6_using_mikrotik = from_python(&py, "findIPv6usingMikrotik").unwrap_or(false);
        
        // Spylnx
        cfg.automatic_import_splynx = from_python(&py, "automaticImportSplynx").unwrap_or(false);
        cfg.splynx_api_key = from_python(&py, "splynx_api_key").unwrap_or("Your API Key Here".to_string());
        cfg.spylnx_api_secret = from_python(&py, "splynx_api_secret").unwrap_or("Your API Secret Here".to_string());
        cfg.spylnx_api_url = from_python(&py, "splynx_api_url").unwrap_or("https://your.splynx.url/api/v1".to_string());

        // UISP
        cfg.automatic_import_uisp = from_python(&py, "automaticImportUISP").unwrap_or(false);
        cfg.uisp_auth_token = from_python(&py, "uispAuthToken").unwrap_or("Your API Token Here".to_string());
        cfg.uisp_base_url = from_python(&py, "UISPbaseURL").unwrap_or("https://your.uisp.url".to_string());
        cfg.uisp_site = from_python(&py, "uispSite").unwrap_or("Your parent site name here".to_string());
        cfg.uisp_strategy = from_python(&py, "uispStrategy").unwrap_or("full".to_string());
        cfg.uisp_suspended_strategy = from_python(&py, "uispSuspendedStrategy").unwrap_or("none".to_string());
        cfg.airmax_capacity = from_python(&py, "airMax_capacity").unwrap_or(0.65);
        cfg.ltu_capacity = from_python(&py, "ltu_capacity").unwrap_or(0.9);
        cfg.bandwidth_overhead_factor = from_python(&py, "bandwidthOverheadFactor").unwrap_or(1.0);
        cfg.committed_bandwidth_multiplier = from_python(&py, "committedBandwidthMultiplier").unwrap_or(0.98);
        cfg.exception_cpes = from_python(&py, "exceptionCPEs").unwrap_or(HashMap::new());

        // API
        cfg.api_username = from_python(&py, "apiUsername").unwrap_or("testUser".to_string());
        cfg.api_password = from_python(&py, "apiPassword").unwrap_or("testPassword".to_string());
        cfg.api_host_ip = from_python(&py, "apiHostIP").unwrap_or("127.0.0.1".to_string());
        cfg.api_host_port = from_python(&py, "apiHostPost").unwrap_or(5000);

        // Powercode
        cfg.automatic_import_powercode = from_python(&py, "automaticImportPowercode").unwrap_or(false);
        cfg.powercode_api_key = from_python(&py,"powercode_api_key").unwrap_or("".to_string());
        cfg.powercode_api_url = from_python(&py,"powercode_api_url").unwrap_or("".to_string());

        // Sonar
        cfg.automatic_import_sonar = from_python(&py, "automaticImportSonar").unwrap_or(false);
        cfg.sonar_api_key = from_python(&py, "sonar_api_key").unwrap_or("".to_string());
        cfg.sonar_api_url = from_python(&py, "sonar_api_url").unwrap_or("".to_string());
        cfg.snmp_community = from_python(&py, "snmp_community").unwrap_or("public".to_string());
        cfg.sonar_active_status_ids = from_python(&py, "sonar_active_status_ids").unwrap_or(vec![]);
        cfg.sonar_airmax_ap_model_ids = from_python(&py, "sonar_airmax_ap_model_ids").unwrap_or(vec![]);
        cfg.sonar_ltu_ap_model_ids = from_python(&py, "sonar_ltu_ap_model_ids").unwrap_or(vec![]);        

        // InfluxDB
        cfg.influx_db_enabled = from_python(&py, "influxDBEnabled").unwrap_or(false);
        cfg.influx_db_url = from_python(&py, "influxDBurl").unwrap_or("http://localhost:8086".to_string());
        cfg.infux_db_bucket = from_python(&py, "influxDBBucket").unwrap_or("libreqos".to_string());
        cfg.influx_db_org = from_python(&py, "influxDBOrg").unwrap_or("Your ISP Name Here".to_string());
        cfg.influx_db_token = from_python(&py, "influxDBtoken").unwrap_or("".to_string());

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

    #[allow(dead_code)]
    pub(crate) fn load_from_string(s: &str) -> Result<Self, PythonMigrationError> {
        let mut old_config = Self::default();
        prepare_freethreaded_python();
        Python::with_gil(|py| {
            py.run(s, None, None).unwrap();
            let result = Self::parse(&mut old_config, &py);
            if result.is_err() {
                println!("Error parsing Python config: {:?}", result);
            }
        });

        Ok(old_config)
    }
}

#[cfg(test)]
mod test {
    use super::super::test_data::*;
    use super::*;

    #[test]
    fn test_parsing_the_default() {
        let mut cfg = PythonMigration::default();
        prepare_freethreaded_python();
        let mut worked = true;
        Python::with_gil(|py| {
            py.run(PYTHON_CONFIG, None, None).unwrap();
            let result = PythonMigration::parse(&mut cfg, &py);
            if result.is_err() {
                println!("Error parsing Python config: {:?}", result);
                worked = false;
            }
        });
        assert!(worked)
    }
}
