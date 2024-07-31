//! Support for Long-Term Statistics, protocol version 2

mod lts_status;
mod lts_client;

use std::thread;
use anyhow::Result;
use log::warn;
use tokio::spawn;
use lqos_config::load_config;
pub use lts_status::{get_lts_status, get_lts_trial_days_remaining};

fn get_lts_config() -> (String, String, String) {
    let config = load_config().unwrap();
    let key = config.long_term_stats.license_key.unwrap_or(String::new());
    let cert = config.long_term_stats.lts_root_pem.unwrap();
    let domain = config.long_term_stats.lts_url.unwrap();
    (key, cert, domain)
}

#[repr(C)]
#[no_mangle]
#[derive(Debug)]
pub struct Lts2Config {
    /// The path to the root certificate for the LTS server
    pub path_to_certificate: Option<String>,
    /// The domain name of the LTS server
    pub domain: Option<String>,
    /// The license key for the LTS server
    pub license_key: Option<String>,
}


fn config_function() -> Lts2Config {
    Lts2Config {
        path_to_certificate: None,
        domain: None,
        license_key: None,
    }
}

pub async fn start_lts2() -> Result<()> {
    // Spawn a thread in which to launch LTS2
    thread::spawn(|| {
        let lib = unsafe {
            libloading::Library::new("/home/herbert/Rust/LibreQoS/lts2/rust/lts2_client/target/debug/liblts2_client.so").unwrap()
        };
        let start_lts2: libloading::Symbol<unsafe extern fn(fn() -> Lts2Config)> = unsafe {
            lib.get(b"start_lts2").unwrap()
        };

        unsafe {
            start_lts2(config_function);
        }
        warn!("LTS2 has exited");
    });
    log::info!("Staring Long-Term Stats 2 Support");

    spawn(lts_status::poll_lts_status());

    Ok(())
}