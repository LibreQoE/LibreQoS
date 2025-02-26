use crate::num_possible_cpus;
use libbpf_sys::bpf_map_update_elem;
use std::ffi::c_void;
use thiserror::Error;
use tracing::error;

#[derive(Default)]
#[repr(C)]
struct TxqConfig {
    /* lookup key: __u32 cpu; */
    queue_mapping: u16,
    htb_major: u16,
}

pub fn map_txq_config_base_setup(map_fd: i32) -> Result<(), MapTxqConfigError> {
    let possible_cpus = num_possible_cpus().map_err(|_| MapTxqConfigError::NumCpusError)?;
    if map_fd < 0 {
        error!("ERR: (bad map_fd:{map_fd}) cannot proceed without access to txq_config map");
        return Err(MapTxqConfigError::BadMapFd);
    }

    let mut txq_cfg = TxqConfig::default();
    for cpu in 0..possible_cpus {
        let cpu_u16: u16 = cpu as u16;
        txq_cfg.queue_mapping = cpu_u16 + 1;
        txq_cfg.htb_major = cpu_u16 + 1;

        let key_ptr: *const u32 = &cpu;
        let val_ptr: *const TxqConfig = &txq_cfg;

        let err = unsafe {
            bpf_map_update_elem(map_fd, key_ptr as *const c_void, val_ptr as *mut c_void, 0)
        };
        if err != 0 {
            error!("Unable to update TXQ map");
            return Err(MapTxqConfigError::BpfMapUpdateFail);
        }
    }

    Ok(())
}

#[derive(Error, Debug)]
pub enum MapTxqConfigError {
    #[error("Unable to determine number of CPUs")]
    NumCpusError,
    #[error("Bad Mapped File Descriptor")]
    BadMapFd,
    #[error("Unable to insert into map")]
    BpfMapUpdateFail,
}
