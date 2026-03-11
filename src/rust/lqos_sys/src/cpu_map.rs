use crate::{linux::map_txq_config_shaping, num_possible_cpus};
use anyhow::{Error, Result};
use libbpf_sys::{bpf_map_update_elem, bpf_obj_get};
use std::{ffi::CString, os::raw::c_void};
use tracing::debug;

//* Provides an interface for querying the number of CPUs eBPF can
//* see, and marking CPUs as available. Callers may provide a shaping
//* CPU allowlist (e.g. exclude E-cores on hybrid CPUs).

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

    pub(crate) fn mark_cpus_available(&self, shaping_physical_cpus: &[u32]) -> Result<()> {
        let cpu_count = num_possible_cpus()?;

        let queue_size = 2048u32;
        let val_ptr: *const u32 = &queue_size;

        // Determine which physical CPUs we will redirect to.
        let mut shaping: Vec<u32> = if shaping_physical_cpus.is_empty() {
            (0..cpu_count).collect()
        } else {
            shaping_physical_cpus
                .iter()
                .copied()
                .filter(|c| *c < cpu_count)
                .collect()
        };
        shaping.sort_unstable();
        shaping.dedup();
        if shaping.is_empty() {
            shaping = (0..cpu_count).collect();
        }

        // Populate CPU map entries for each destination CPU (physical CPU IDs).
        for cpu_dest in shaping.iter().copied() {
            debug!("Mapping destination CPU #{cpu_dest} into cpumap");
            let cpu_ptr: *const u32 = &cpu_dest;
            let error = unsafe {
                bpf_map_update_elem(
                    self.fd_cpu_map,
                    cpu_ptr as *const c_void,
                    val_ptr as *const c_void,
                    0,
                )
            };
            if error != 0 {
                return Err(Error::msg("Unable to map CPU in cpumap"));
            }
        }

        // Populate logical->physical mapping table for ALL possible CPU indices.
        // This prevents stale mappings from implicitly redirecting to CPU0 due to
        // uninitialized array values.
        let shaping_len = shaping.len() as u32;
        for logical_cpu in 0..cpu_count {
            let cpu_dest = shaping[(logical_cpu % shaping_len) as usize];
            debug!("Mapping logical CPU #{logical_cpu} -> physical CPU #{cpu_dest}");
            let key_ptr: *const u32 = &logical_cpu;
            let val_ptr: *const u32 = &cpu_dest;
            let error = unsafe {
                bpf_map_update_elem(
                    self.fd_cpu_available,
                    key_ptr as *const c_void,
                    val_ptr as *const c_void,
                    0,
                )
            };
            if error != 0 {
                return Err(Error::msg("Unable to add to available CPUs list"));
            }
        }
        Ok(())
    }

    pub(crate) fn setup_base_txq_config(&self, shaping_physical_cpus: &[u32]) -> Result<()> {
        Ok(map_txq_config_shaping(
            self.fd_txq_config,
            shaping_physical_cpus,
        )?)
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
    debug!("xps_setup");
    let queues = sorted_txq_xps_cpus(interface)?;
    for (cpu, xps_cpu) in queues.iter().enumerate() {
        let mask = cpu_to_mask_disabled(cpu);
        let mut f = std::fs::OpenOptions::new().write(true).open(xps_cpu)?;
        f.write_all(mask.to_string().as_bytes())?;
        f.flush()?;
        debug!("Mapped TX queue for CPU {cpu}");
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
                        filename.to_str().unwrap_or_default()
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
