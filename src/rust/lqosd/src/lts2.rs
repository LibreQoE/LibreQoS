//! Support for Long-Term Statistics, protocol version 2

mod shared_types;
mod lts_status;

use std::path::Path;
use std::sync::atomic::AtomicI32;
use std::thread;
use anyhow::Result;
use log::warn;
use lqos_config::load_config;
pub use crate::lts2::lts_status::{LtsStatus};
use crate::lts2::shared_types::{ControlReceiver, GetConfigFn, SendStatusFn};
pub use shared_types::{ControlSender, FreeTrialDetails, LtsCommand};

#[no_mangle]
fn config_function(cfg: &mut shared_types::Lts2Config) {
    if let Ok(config) = load_config() {
        cfg.path_to_certificate = config.long_term_stats.lts_root_pem;
        cfg.domain =  config.long_term_stats.lts_url;
        cfg.license_key = config.long_term_stats.license_key;
        cfg.node_id = config.node_id;
    }
}

static LTS_STATUS: AtomicI32 = AtomicI32::new(LtsStatus::NotChecked as i32);
static LTS_TRIAL_EXPIRATION: AtomicI32 = AtomicI32::new(0);

#[no_mangle]
fn set_status_function(valid: bool, status_code: i32, expiration: i32) {
    if !valid {
        LTS_STATUS.store(LtsStatus::Invalid as i32, std::sync::atomic::Ordering::Relaxed);
    } else {
        LTS_STATUS.store(status_code, std::sync::atomic::Ordering::Relaxed);
        if status_code == LtsStatus::FreeTrial as i32 {
            LTS_TRIAL_EXPIRATION.store(expiration, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

pub fn get_lts_status() -> (LtsStatus, Option<i32>) {
    let status = LTS_STATUS.load(std::sync::atomic::Ordering::Relaxed);
    let expiration = if status == LtsStatus::FreeTrial as i32 {
        Some(LTS_TRIAL_EXPIRATION.load(std::sync::atomic::Ordering::Relaxed))
    } else {
        None
    };
    (LtsStatus::from_i32(status), expiration)
}

pub async fn start_lts2() -> Result<shared_types::ControlSender> {
    // Check that we can load the library
    let config = load_config()?;
    let path = Path::new(&config.lqos_directory)
        .join("bin")
        .join("liblts2_client.so");
    if !path.exists() {
        log::error!("Could not find LTS2 client library at {:?}", path);
        return Err(anyhow::anyhow!("Could not find LTS2 client library at {:?}", path));
    }

    let (tx, rx) = std::sync::mpsc::channel::<shared_types::LtsCommand>();

    // Spawn a thread in which to launch LTS2
    thread::spawn(move || {
        let lib = unsafe {
            libloading::Library::new(path).unwrap()
        };
        let start_lts2: libloading::Symbol<unsafe extern fn(GetConfigFn, SendStatusFn, ControlReceiver)> = unsafe {
            lib.get(b"start_lts2").unwrap()
        };

        unsafe {
            start_lts2(config_function, set_status_function, rx);
        }
        warn!("LTS2 has exited");
    });

    Ok(tx)
}