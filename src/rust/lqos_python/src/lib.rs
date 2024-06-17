use lqos_bus::{BusRequest, BusResponse, TcHandle};
use lqos_utils::hex_string::read_hex_string;
use nix::libc::getpid;
use pyo3::{
  exceptions::PyOSError, pyclass, pyfunction, pymethods, pymodule,
  types::PyModule, wrap_pyfunction, PyResult, Python,
};
use std::{
  fs::{remove_file, File},
  io::Write,
  path::Path,
};
mod blocking;
use anyhow::{Error, Result};
use blocking::run_query;
use sysinfo::System;
mod device_weights;

const LOCK_FILE: &str = "/run/lqos/libreqos.lock";

/// Defines the Python module exports.
/// All exported functions have to be listed here.
#[pymodule]
fn liblqos_python(_py: Python, m: &PyModule) -> PyResult<()> {
  m.add_class::<PyIpMapping>()?;
  m.add_class::<BatchedCommands>()?;
  m.add_class::<PyExceptionCpe>()?;
  m.add_class::<device_weights::DeviceWeightResponse>()?;
  m.add_wrapped(wrap_pyfunction!(is_lqosd_alive))?;
  m.add_wrapped(wrap_pyfunction!(list_ip_mappings))?;
  m.add_wrapped(wrap_pyfunction!(clear_ip_mappings))?;
  m.add_wrapped(wrap_pyfunction!(delete_ip_mapping))?;
  m.add_wrapped(wrap_pyfunction!(add_ip_mapping))?;
  m.add_wrapped(wrap_pyfunction!(validate_shaped_devices))?;
  m.add_wrapped(wrap_pyfunction!(is_libre_already_running))?;
  m.add_wrapped(wrap_pyfunction!(create_lock_file))?;
  m.add_wrapped(wrap_pyfunction!(free_lock_file))?;
  // Unified configuration items
  m.add_wrapped(wrap_pyfunction!(check_config))?;
  m.add_wrapped(wrap_pyfunction!(sqm))?;
  m.add_wrapped(wrap_pyfunction!(upstream_bandwidth_capacity_download_mbps))?;
  m.add_wrapped(wrap_pyfunction!(upstream_bandwidth_capacity_upload_mbps))?;
  m.add_wrapped(wrap_pyfunction!(interface_a))?;
  m.add_wrapped(wrap_pyfunction!(interface_b))?;
  m.add_wrapped(wrap_pyfunction!(enable_actual_shell_commands))?;
  m.add_wrapped(wrap_pyfunction!(use_bin_packing_to_balance_cpu))?;
  m.add_wrapped(wrap_pyfunction!(monitor_mode_only))?;
  m.add_wrapped(wrap_pyfunction!(run_shell_commands_as_sudo))?;
  m.add_wrapped(wrap_pyfunction!(generated_pn_download_mbps))?;
  m.add_wrapped(wrap_pyfunction!(generated_pn_upload_mbps))?;
  m.add_wrapped(wrap_pyfunction!(queues_available_override))?;
  m.add_wrapped(wrap_pyfunction!(on_a_stick))?;
  m.add_wrapped(wrap_pyfunction!(overwrite_network_json_always))?;
  m.add_wrapped(wrap_pyfunction!(allowed_subnets))?;
  m.add_wrapped(wrap_pyfunction!(ignore_subnets))?;
  m.add_wrapped(wrap_pyfunction!(circuit_name_use_address))?;
  m.add_wrapped(wrap_pyfunction!(find_ipv6_using_mikrotik))?;
  m.add_wrapped(wrap_pyfunction!(exclude_sites))?;
  m.add_wrapped(wrap_pyfunction!(bandwidth_overhead_factor))?;
  m.add_wrapped(wrap_pyfunction!(committed_bandwidth_multiplier))?;
  m.add_wrapped(wrap_pyfunction!(exception_cpes))?;
  m.add_wrapped(wrap_pyfunction!(uisp_site))?;
  m.add_wrapped(wrap_pyfunction!(uisp_strategy))?;
  m.add_wrapped(wrap_pyfunction!(uisp_suspended_strategy))?;
  m.add_wrapped(wrap_pyfunction!(airmax_capacity))?;
  m.add_wrapped(wrap_pyfunction!(ltu_capacity))?;
  m.add_wrapped(wrap_pyfunction!(use_ptmp_as_parent))?;
  m.add_wrapped(wrap_pyfunction!(uisp_base_url))?;
  m.add_wrapped(wrap_pyfunction!(uisp_auth_token))?;
  m.add_wrapped(wrap_pyfunction!(splynx_api_key))?;
  m.add_wrapped(wrap_pyfunction!(splynx_api_secret))?;
  m.add_wrapped(wrap_pyfunction!(splynx_api_url))?;
  m.add_wrapped(wrap_pyfunction!(automatic_import_uisp))?;
  m.add_wrapped(wrap_pyfunction!(automatic_import_splynx))?;
  m.add_wrapped(wrap_pyfunction!(queue_refresh_interval_mins))?;
  m.add_wrapped(wrap_pyfunction!(automatic_import_powercode))?;
  m.add_wrapped(wrap_pyfunction!(powercode_api_key))?;
  m.add_wrapped(wrap_pyfunction!(powercode_api_url))?;
  m.add_wrapped(wrap_pyfunction!(automatic_import_sonar))?;
  m.add_wrapped(wrap_pyfunction!(sonar_api_url))?;
  m.add_wrapped(wrap_pyfunction!(sonar_api_key))?;
  m.add_wrapped(wrap_pyfunction!(snmp_community))?;
  m.add_wrapped(wrap_pyfunction!(sonar_airmax_ap_model_ids))?;
  m.add_wrapped(wrap_pyfunction!(sonar_ltu_ap_model_ids))?;
  m.add_wrapped(wrap_pyfunction!(sonar_active_status_ids))?;
  m.add_wrapped(wrap_pyfunction!(influx_db_enabled))?;
  m.add_wrapped(wrap_pyfunction!(influx_db_bucket))?;
  m.add_wrapped(wrap_pyfunction!(influx_db_org))?;
  m.add_wrapped(wrap_pyfunction!(influx_db_token))?;
  m.add_wrapped(wrap_pyfunction!(influx_db_url))?;
  m.add_wrapped(wrap_pyfunction!(get_weights))?;
  m.add_wrapped(wrap_pyfunction!(get_tree_weights))?;
  m.add_wrapped(wrap_pyfunction!(get_libreqos_directory))?;

  Ok(())
}

/// Check that `lqosd` is running.
///
/// Returns true if it is running, false otherwise.
#[pyfunction]
fn is_lqosd_alive(_py: Python) -> PyResult<bool> {
  if let Ok(reply) = run_query(vec![BusRequest::Ping]) {
    for resp in reply.iter() {
      if let BusResponse::Ack = resp {
        return Ok(true);
      }
    }
  }
  Ok(false)
}

/// Provides a representation of an IP address mapping
/// Available through python by field name.
#[pyclass]
pub struct PyIpMapping {
  #[pyo3(get)]
  pub ip_address: String,
  #[pyo3(get)]
  pub prefix_length: u32,
  #[pyo3(get)]
  pub tc_handle: (u16, u16),
  #[pyo3(get)]
  pub cpu: u32,
}

/// Returns a list of all IP mappings
#[pyfunction]
fn list_ip_mappings(_py: Python) -> PyResult<Vec<PyIpMapping>> {
  let mut result = Vec::new();
  if let Ok(reply) = run_query(vec![BusRequest::ListIpFlow]) {
    for resp in reply.iter() {
      if let BusResponse::MappedIps(map) = resp {
        for mapping in map.iter() {
          result.push(PyIpMapping {
            ip_address: mapping.ip_address.clone(),
            prefix_length: mapping.prefix_length,
            tc_handle: mapping.tc_handle.get_major_minor(),
            cpu: mapping.cpu,
          });
        }
      }
    }
  }
  Ok(result)
}

/// Clear all IP address to TC/CPU mappings
#[pyfunction]
fn clear_ip_mappings(_py: Python) -> PyResult<()> {
  run_query(vec![BusRequest::ClearIpFlow]).unwrap();
  Ok(())
}

/// Deletes an IP to CPU/TC mapping.
///
/// ## Arguments
///
/// * `ip_address`: The IP address to unmap.
/// * `upload`: `true` if this needs to be applied to the upload map (for a split/stick setup)
#[pyfunction]
fn delete_ip_mapping(_py: Python, ip_address: String) -> PyResult<()> {
  run_query(vec![
    BusRequest::DelIpFlow { ip_address: ip_address.clone(), upload: false },
    BusRequest::DelIpFlow { ip_address, upload: true },
  ])
  .unwrap();
  Ok(())
}

/// Internal function
/// Converts IP address arguments into an IP mapping request.
fn parse_add_ip(
  ip: &str,
  classid: &str,
  cpu: &str,
  upload: bool,
) -> Result<BusRequest> {
  if !classid.contains(':') {
    return Err(Error::msg(
      format!("Class id must be in the format (major):(minor), e.g. 1:12. Provided string: {classid}"),
    ));
  }
  Ok(BusRequest::MapIpToFlow {
    ip_address: ip.to_string(),
    tc_handle: TcHandle::from_string(classid)?,
    cpu: read_hex_string(cpu)?, // Force HEX representation
    upload,
  })
}

/// Adds an IP address mapping
#[pyfunction]
fn add_ip_mapping(
  ip: String,
  classid: String,
  cpu: String, // In HEX
  upload: bool,
) -> PyResult<()> {
  let request = parse_add_ip(&ip, &classid, &cpu, upload);
  if let Ok(request) = request {
    run_query(vec![request]).unwrap();
    Ok(())
  } else {
    Err(PyOSError::new_err(request.err().unwrap().to_string()))
  }
}

#[pyclass]
pub struct BatchedCommands {
  batch: Vec<BusRequest>,
}

#[pymethods]
impl BatchedCommands {
  #[new]
  pub fn new() -> PyResult<Self> {
    Ok(Self { batch: Vec::new() })
  }

  pub fn add_ip_mapping(
    &mut self,
    ip: String,
    classid: String,
    cpu: String,
    upload: bool,
  ) -> PyResult<()> {
    let request = parse_add_ip(&ip, &classid, &cpu, upload);
    if let Ok(request) = request {
      self.batch.push(request);
      Ok(())
    } else {
      Err(PyOSError::new_err(request.err().unwrap().to_string()))
    }
  }

  pub fn length(&self) -> PyResult<usize> {
    Ok(self.batch.len())
  }

  pub fn log(&self) -> PyResult<()> {
    self.batch.iter().for_each(|c| println!("{c:?}"));
    Ok(())
  }

  pub fn submit(&mut self) -> PyResult<usize> {
    const MAX_BATH_SIZE: usize = 512;
    // We're draining the request list out, which is a move that
    // *should* be elided by the optimizing compiler.
    let len = self.batch.len();
    while !self.batch.is_empty() {
      let batch_size = usize::min(MAX_BATH_SIZE, self.batch.len());
      let batch: Vec<BusRequest> = self.batch.drain(0..batch_size).collect();
      run_query(batch).unwrap();
    }
    Ok(len)
  }
}

/// Requests Rust-side validation of `ShapedDevices.csv`
#[pyfunction]
fn validate_shaped_devices() -> PyResult<String> {
  let result = run_query(vec![BusRequest::ValidateShapedDevicesCsv]).unwrap();
  for response in result.iter() {
    match response {
      BusResponse::Ack => return Ok("OK".to_string()),
      BusResponse::ShapedDevicesValidation(error) => return Ok(error.clone()),
      _ => {}
    }
  }
  Ok("".to_string())
}

#[pyfunction]
fn is_libre_already_running() -> PyResult<bool> {
  let lock_path = Path::new(LOCK_FILE);
  if lock_path.exists() {
    let contents = std::fs::read_to_string(lock_path);
    if let Ok(contents) = contents {
      if let Ok(pid) = contents.parse::<i32>() {
        let sys = System::new_all();
        let pid = sysinfo::Pid::from(pid as usize);
        if let Some(process) = sys.processes().get(&pid) {
          if process.name().contains("python") {
            return Ok(true);
          }
        }
      } else {
        println!("{LOCK_FILE} did not contain a valid PID");
        return Ok(false);
      }
    } else {
      println!("Error reading contents of {LOCK_FILE}");
      return Ok(false);
    }
  }
  Ok(false)
}

#[pyfunction]
fn create_lock_file() -> PyResult<()> {
  let pid = unsafe { getpid() };
  let pid_format = format!("{pid}");
  {
    if let Ok(mut f) = File::create(LOCK_FILE) {
      f.write_all(pid_format.as_bytes())?;
    }
  }
  Ok(())
}

#[pyfunction]
fn free_lock_file() -> PyResult<()> {
  let _ = remove_file(LOCK_FILE); // Ignore result
  Ok(())
}

#[pyfunction]
fn check_config() -> PyResult<bool> {
  let config = lqos_config::load_config();
  if let Err(e) = config {
    println!("Error loading config: {e}");
    return Ok(false);
  }
  Ok(true)
}

#[pyfunction]
fn sqm() -> PyResult<String> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.queues.default_sqm.clone())
}

#[pyfunction]
fn upstream_bandwidth_capacity_download_mbps() -> PyResult<u32> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.queues.uplink_bandwidth_mbps)
}

#[pyfunction]
fn upstream_bandwidth_capacity_upload_mbps() -> PyResult<u32> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.queues.uplink_bandwidth_mbps)
}

#[pyfunction]
fn interface_a() -> PyResult<String> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.isp_interface())
}

#[pyfunction]
fn interface_b() -> PyResult<String> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.internet_interface())
}

#[pyfunction]
fn enable_actual_shell_commands() -> PyResult<bool> {
  let config = lqos_config::load_config().unwrap();
  Ok(!config.queues.dry_run)
}

#[pyfunction]
fn use_bin_packing_to_balance_cpu() -> PyResult<bool> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.queues.use_binpacking)
}

#[pyfunction]
fn monitor_mode_only() -> PyResult<bool> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.queues.monitor_only)
}

#[pyfunction]
fn run_shell_commands_as_sudo() -> PyResult<bool> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.queues.sudo)
}

#[pyfunction]
fn generated_pn_download_mbps() -> PyResult<u32> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.queues.generated_pn_download_mbps)
}

#[pyfunction]
fn generated_pn_upload_mbps() -> PyResult<u32> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.queues.generated_pn_upload_mbps)
}

#[pyfunction]
fn queues_available_override() -> PyResult<u32> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.queues.override_available_queues.unwrap_or(0))
}

#[pyfunction]
fn on_a_stick() -> PyResult<bool> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.on_a_stick_mode())
}

#[pyfunction]
fn overwrite_network_json_always() -> PyResult<bool> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.integration_common.always_overwrite_network_json)
}

#[pyfunction]
fn allowed_subnets() -> PyResult<Vec<String>> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.ip_ranges.allow_subnets.clone())
}

#[pyfunction]
fn ignore_subnets() -> PyResult<Vec<String>> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.ip_ranges.ignore_subnets.clone())
}

#[pyfunction]
fn circuit_name_use_address() -> PyResult<bool> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.integration_common.circuit_name_as_address)
}

#[pyfunction]
fn find_ipv6_using_mikrotik() -> PyResult<bool> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.uisp_integration.ipv6_with_mikrotik)
}

#[pyfunction]
fn exclude_sites() -> PyResult<Vec<String>> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.uisp_integration.exclude_sites.clone())
}

#[pyfunction]
fn bandwidth_overhead_factor() -> PyResult<f32> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.uisp_integration.bandwidth_overhead_factor)
}

#[pyfunction]
fn committed_bandwidth_multiplier() -> PyResult<f32> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.uisp_integration.commit_bandwidth_multiplier)
}

#[pyclass]
pub struct PyExceptionCpe {
  pub cpe: String,
  pub parent: String,
}

#[pyfunction]
fn exception_cpes() -> PyResult<Vec<PyExceptionCpe>> {
  let config = lqos_config::load_config().unwrap();
  let mut result = Vec::new();
  for cpe in config.uisp_integration.exception_cpes.iter() {
    result.push(PyExceptionCpe {
      cpe: cpe.cpe.clone(),
      parent: cpe.parent.clone(),
    });
  }
  Ok(result)
}

#[pyfunction]
fn uisp_site() -> PyResult<String> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.uisp_integration.site)
}

#[pyfunction]
fn uisp_strategy() -> PyResult<String> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.uisp_integration.strategy)
}

#[pyfunction]
fn uisp_suspended_strategy() -> PyResult<String> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.uisp_integration.suspended_strategy)
}

#[pyfunction]
fn airmax_capacity() -> PyResult<f32> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.uisp_integration.airmax_capacity)
}

#[pyfunction]
fn ltu_capacity() -> PyResult<f32> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.uisp_integration.ltu_capacity)
}

#[pyfunction]
fn use_ptmp_as_parent() -> PyResult<bool> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.uisp_integration.use_ptmp_as_parent)
}

#[pyfunction]
fn uisp_base_url() -> PyResult<String> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.uisp_integration.url)
}

#[pyfunction]
fn uisp_auth_token() -> PyResult<String> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.uisp_integration.token)
}

#[pyfunction]
fn splynx_api_key() -> PyResult<String> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.spylnx_integration.api_key)
}

#[pyfunction]
fn splynx_api_secret() -> PyResult<String> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.spylnx_integration.api_secret)
}

#[pyfunction]
fn splynx_api_url() -> PyResult<String> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.spylnx_integration.url)
}

#[pyfunction]
fn automatic_import_uisp() -> PyResult<bool> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.uisp_integration.enable_uisp)
}

#[pyfunction]
fn automatic_import_splynx() -> PyResult<bool> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.uisp_integration.enable_uisp)
}

#[pyfunction]
fn queue_refresh_interval_mins() -> PyResult<u32> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.integration_common.queue_refresh_interval_mins)
}

#[pyfunction]
fn automatic_import_powercode() -> PyResult<bool> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.powercode_integration.enable_powercode)
}

#[pyfunction]
fn powercode_api_key() -> PyResult<String> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.powercode_integration.powercode_api_key)
}

#[pyfunction]
fn powercode_api_url() -> PyResult<String> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.powercode_integration.powercode_api_url)
}

#[pyfunction]
fn automatic_import_sonar() -> PyResult<bool> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.sonar_integration.enable_sonar)
}

#[pyfunction]
fn sonar_api_url() -> PyResult<String> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.sonar_integration.sonar_api_url)
}

#[pyfunction]
fn sonar_api_key() -> PyResult<String> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.sonar_integration.sonar_api_key)
}

#[pyfunction]
fn snmp_community() -> PyResult<String> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.sonar_integration.snmp_community)
}

#[pyfunction]
fn sonar_airmax_ap_model_ids() -> PyResult<Vec<String>> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.sonar_integration.airmax_model_ids)
}

#[pyfunction]
fn sonar_ltu_ap_model_ids() -> PyResult<Vec<String>> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.sonar_integration.ltu_model_ids)
}

#[pyfunction]
fn sonar_active_status_ids() -> PyResult<Vec<String>> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.sonar_integration.active_status_ids)
}

#[pyfunction]
fn influx_db_enabled() -> PyResult<bool> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.influxdb.enable_influxdb)
}

#[pyfunction]
fn influx_db_bucket() -> PyResult<String> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.influxdb.bucket)
}

#[pyfunction]
fn influx_db_org() -> PyResult<String> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.influxdb.org)
}

#[pyfunction]
fn influx_db_token() -> PyResult<String> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.influxdb.token)
}

#[pyfunction]
fn influx_db_url() -> PyResult<String> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.influxdb.url)
}

#[pyfunction]
pub fn get_weights() -> PyResult<Vec<device_weights::DeviceWeightResponse>> {
    match device_weights::get_weights_rust() {
        Ok(weights) => Ok(weights),
        Err(e) => {
            Err(PyOSError::new_err(e.to_string()))
        }
    }
}

#[pyfunction]
pub fn get_tree_weights() -> PyResult<Vec<device_weights::NetworkNodeWeight>> {
    match device_weights::calculate_tree_weights() {
        Ok(w) => Ok(w),
        Err(e) => {
            Err(PyOSError::new_err(e.to_string()))
        }
    }
}

#[pyfunction]
pub fn get_libreqos_directory() -> PyResult<String> {
  let config = lqos_config::load_config().unwrap();
  Ok(config.lqos_directory)
}