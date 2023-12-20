use anyhow::{Error, Result};
use libbpf_sys::{bpf_map_update_elem, bpf_obj_get};
use log::info;
use std::{ffi::CString, os::raw::c_void};
use crate::{num_possible_cpus, linux::map_txq_config_base_setup};

//* Provides an interface for querying the number of CPUs eBPF can
//* see, and marking CPUs as available. Currently marks ALL eBPF
//* usable CPUs as available.

pub(crate) struct CpuMapping {
  fd_cpu_map: i32,
  fd_cpu_available: i32,
  fd_txq_config: i32,
}

fn get_map_fd(filename: &str) -> Result<i32> {
  let filename_c = CString::new(filename)?;
  let fd = unsafe { bpf_obj_get(filename_c.as_ptr()) };
  if fd < 0 {
    Err(Error::msg("Unable to open BPF map"))
  } else {
    Ok(fd)
  }
}

impl CpuMapping {
  pub(crate) fn new() -> Result<Self> {
    Ok(Self {
      fd_cpu_map: get_map_fd("/sys/fs/bpf/cpu_map")?,
      fd_cpu_available: get_map_fd("/sys/fs/bpf/cpus_available")?,
      fd_txq_config: get_map_fd("/sys/fs/bpf/map_txq_config")?,
    })
  }

  pub(crate) fn mark_cpus_available(&self) -> Result<()> {
    let cpu_count = num_possible_cpus()?;

    let queue_size = 2048u32;
    let val_ptr: *const u32 = &queue_size;
    for cpu in 0..cpu_count {
      info!("Mapping core #{cpu}");
      // Insert into the cpu map
      let cpu_ptr: *const u32 = &cpu;
      let error = unsafe {
        bpf_map_update_elem(
          self.fd_cpu_map,
          cpu_ptr as *const c_void,
          val_ptr as *const c_void,
          0,
        )
      };
      if error != 0 {
        return Err(Error::msg("Unable to map CPU"));
      }

      // Insert into the available list
      let error = unsafe {
        bpf_map_update_elem(
          self.fd_cpu_available,
          cpu_ptr as *const c_void,
          cpu_ptr as *const c_void,
          0,
        )
      };
      if error != 0 {
        return Err(Error::msg("Unable to add to available CPUs list"));
      }
    } // CPU loop
    Ok(())
  }

  pub(crate) fn setup_base_txq_config(&self) -> Result<()> {
    Ok(map_txq_config_base_setup(self.fd_txq_config)?)
  }
}

impl Drop for CpuMapping {
  fn drop(&mut self) {
    let _ = nix::unistd::close(self.fd_cpu_available);
    let _ = nix::unistd::close(self.fd_cpu_map);
    let _ = nix::unistd::close(self.fd_txq_config);
  }
}

/// Emulates xd_setup from cpumap
pub(crate) fn xps_setup_default_disable(interface: &str) -> Result<()> {
  use std::io::Write;
  info!("xps_setup");
  let queues = sorted_txq_xps_cpus(interface)?;
  for (cpu, xps_cpu) in queues.iter().enumerate() {
    let mask = cpu_to_mask_disabled(cpu);
    let mut f = std::fs::OpenOptions::new().write(true).open(xps_cpu)?;
    f.write_all(mask.to_string().as_bytes())?;
    f.flush()?;
    info!("Mapped TX queue for CPU {cpu}");
  }

  Ok(())
}

fn sorted_txq_xps_cpus(interface: &str) -> Result<Vec<String>> {
  let mut result = Vec::new();
  let paths =
    std::fs::read_dir(&format!("/sys/class/net/{interface}/queues/"))
      .map_err(|_| anyhow::anyhow!("/sys/class/net/{interface}/queues/ does not exist. Does this card only support one queue (not supported)?"))?;
  for path in paths {
    if let Ok(path) = &path {
      if path.path().is_dir() {
        if let Some(filename) = path.path().file_name() {
          let base_fn = format!(
            "/sys/class/net/{interface}/queues/{}/xps_cpus",
            filename.to_str().unwrap()
          );
          if std::path::Path::new(&base_fn).exists() {
            result.push(base_fn);
          }
        }
      }
    }
  }
  result.sort();

  Ok(result)
}

fn cpu_to_mask_disabled(_cpu: usize) -> usize {
  0
}
