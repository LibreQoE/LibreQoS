//! Control-map helpers for temporarily bypassing XDP redirection and TC classification.

use crate::lqos_kernel::bpf::libbpf_num_possible_cpus;
use anyhow::{Error, Result};
use libbpf_sys::{bpf_map_update_elem, bpf_obj_get};
use nix::libc::close;
use std::{
    ffi::{CString, c_void},
    io,
    mem::size_of,
};
use tracing::error;

const TC_CLASSIFY_CONTROL_PATH: &str = "/sys/fs/bpf/tc_classify_control";
const TC_CLASSIFY_CONTROL_KEY: u32 = 0;
const PER_CPU_VALUE_ALIGNMENT: usize = 8;

/// Rust mirror of `struct tc_classify_control` in the eBPF program.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct TcClassifyControl {
    /// Non-zero tells XDP and TC classify to pass packets without redirecting
    /// or setting TC handles.
    pub bypass: u32,
}

fn aligned_per_cpu_value_size() -> usize {
    size_of::<TcClassifyControl>().div_ceil(PER_CPU_VALUE_ALIGNMENT) * PER_CPU_VALUE_ALIGNMENT
}

fn per_cpu_payload(enabled: bool, cpu_count: u32) -> Vec<u8> {
    let stride = aligned_per_cpu_value_size();
    let mut payload = vec![0u8; stride * cpu_count as usize];
    let control = TcClassifyControl {
        bypass: u32::from(enabled),
    };
    let bytes = control.bypass.to_ne_bytes();
    for cpu in 0..cpu_count as usize {
        let offset = cpu * stride;
        payload[offset..offset + bytes.len()].copy_from_slice(&bytes);
    }
    payload
}

fn update_tc_classify_control(enabled: bool) -> Result<()> {
    let path_c = CString::new(TC_CLASSIFY_CONTROL_PATH)?;
    let fd = unsafe { bpf_obj_get(path_c.as_ptr()) };
    if fd < 0 {
        let error = io::Error::last_os_error();
        let errno = error.raw_os_error().unwrap_or(0);
        return Err(Error::msg(format!(
            "Unable to open BPF map '{TC_CLASSIFY_CONTROL_PATH}' for TC classify bypass update (fd={fd}, errno={errno}, error={error})"
        )));
    }

    let cpu_count = unsafe { libbpf_num_possible_cpus() };
    if cpu_count <= 0 {
        unsafe {
            close(fd);
        }
        return Err(Error::msg(format!(
            "Unable to determine CPU count for TC classify bypass update: libbpf_num_possible_cpus returned {cpu_count}"
        )));
    }
    let cpu_count = u32::try_from(cpu_count).map_err(|e| {
        unsafe {
            close(fd);
        }
        Error::msg(format!(
            "Unable to convert CPU count for TC classify bypass update: {e}"
        ))
    })?;
    let mut key = TC_CLASSIFY_CONTROL_KEY;
    let payload = per_cpu_payload(enabled, cpu_count);
    let err = unsafe {
        bpf_map_update_elem(
            fd,
            &mut key as *mut u32 as *mut c_void,
            payload.as_ptr() as *mut c_void,
            0,
        )
    };
    let update_error = (err != 0).then(io::Error::last_os_error);
    unsafe {
        close(fd);
    }
    if err != 0 {
        let error = update_error.unwrap_or_else(|| io::Error::from_raw_os_error(0));
        let errno = error.raw_os_error().unwrap_or(0);
        error!(
            "Unable to update BPF map '{}' for TC classify bypass={} (err={}, errno={}, error={})",
            TC_CLASSIFY_CONTROL_PATH, enabled, err, errno, error
        );
        return Err(Error::msg(format!(
            "Unable to update BPF map '{TC_CLASSIFY_CONTROL_PATH}' for TC classify bypass={enabled} (err={err}, errno={errno}, error={error})"
        )));
    }

    Ok(())
}

/// Initializes TC classify bypass to disabled.
///
/// This function has side effects: it writes all per-CPU values for the pinned
/// `tc_classify_control` BPF map.
pub fn initialize_tc_classify_bypass() -> Result<()> {
    update_tc_classify_control(false)
}

/// Enables or disables TC classify bypass.
///
/// This function has side effects: it writes all per-CPU values for the pinned
/// `tc_classify_control` BPF map. When enabled, XDP returns `XDP_PASS` before
/// CPU redirection, and TC classify returns `TC_ACT_OK` before consuming XDP
/// metadata, flow cache, hot cache, or LPM mappings.
pub fn set_tc_classify_bypass(enabled: bool) -> Result<()> {
    update_tc_classify_control(enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_cpu_payload_uses_eight_byte_stride() {
        let payload = per_cpu_payload(true, 3);
        assert_eq!(payload.len(), 3 * PER_CPU_VALUE_ALIGNMENT);
        for cpu in 0..3 {
            let offset = cpu * PER_CPU_VALUE_ALIGNMENT;
            assert_eq!(
                &payload[offset..offset + size_of::<u32>()],
                &1u32.to_ne_bytes()
            );
            assert_eq!(
                &payload[offset + size_of::<u32>()..offset + PER_CPU_VALUE_ALIGNMENT],
                &[0, 0, 0, 0]
            );
        }
    }

    #[test]
    fn disabled_payload_clears_all_per_cpu_values() {
        let payload = per_cpu_payload(false, 2);
        assert_eq!(payload, vec![0u8; 2 * PER_CPU_VALUE_ALIGNMENT]);
    }
}
