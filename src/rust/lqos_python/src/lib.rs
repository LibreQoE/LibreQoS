use lqos_bus::{BusRequest, BusResponse, TcHandle};
use nix::libc::getpid;
use pyo3::{
  exceptions::PyOSError, pyclass, pyfunction, pymodule, types::PyModule,
  wrap_pyfunction, PyResult, Python,
};
use std::{
  fs::{remove_file, File},
  io::Write,
  path::Path,
};
mod blocking;
use anyhow::{Error, Result};
use blocking::run_query;
use sysinfo::{ProcessExt, System, SystemExt};

const LOCK_FILE: &str = "/run/lqos/libreqos.lock";

/// Defines the Python module exports.
/// All exported functions have to be listed here.
#[pymodule]
fn liblqos_python(_py: Python, m: &PyModule) -> PyResult<()> {
  m.add_class::<PyIpMapping>()?;
  m.add_wrapped(wrap_pyfunction!(is_lqosd_alive))?;
  m.add_wrapped(wrap_pyfunction!(list_ip_mappings))?;
  m.add_wrapped(wrap_pyfunction!(clear_ip_mappings))?;
  m.add_wrapped(wrap_pyfunction!(delete_ip_mapping))?;
  m.add_wrapped(wrap_pyfunction!(add_ip_mapping))?;
  m.add_wrapped(wrap_pyfunction!(validate_shaped_devices))?;
  m.add_wrapped(wrap_pyfunction!(is_libre_already_running))?;
  m.add_wrapped(wrap_pyfunction!(create_lock_file))?;
  m.add_wrapped(wrap_pyfunction!(free_lock_file))?;
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
      "Class id must be in the format (major):(minor), e.g. 1:12",
    ));
  }
  Ok(BusRequest::MapIpToFlow {
    ip_address: ip.to_string(),
    tc_handle: TcHandle::from_string(classid)?,
    cpu: u32::from_str_radix(&cpu.replace("0x", ""), 16)?, // Force HEX representation
    upload,
  })
}

/// Adds an IP address mapping
#[pyfunction]
fn add_ip_mapping(
  ip: String,
  classid: String,
  cpu: String,
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
