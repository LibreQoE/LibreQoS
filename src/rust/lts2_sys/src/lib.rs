use std::ffi::CString;
use std::ptr::null_mut;
use log::error;
use lqos_config::load_config;
mod external;
pub mod shared_types;

use anyhow::{bail, Result};
use crate::shared_types::{FreeTrialDetails, LtsStatus};

pub fn start_lts2() -> Result<()> {
    // Launch the process
    let cfg = get_config()?;
    unsafe {
        external::spawn_lts2(
            cfg.has_remote_host,
            cfg.remote_host.as_ptr(),
            cfg.license_key.as_ptr(),
            cfg.node_id.as_ptr(),
            cfg.node_name.as_ptr(),
        );
    }

    Ok(())
}

pub fn update_config() -> Result<()> {
    let cfg = get_config()?;
    unsafe {
        if external::update_license_status(
            cfg.has_remote_host,
            cfg.remote_host.as_ptr(),
            cfg.license_key.as_ptr(),
            cfg.node_id.as_ptr(),
            cfg.node_name.as_ptr(),
        ) != 0 {
            anyhow::bail!("Failed to update license status");
        } else {
            Ok(())
        }
    }
}

pub fn request_free_trial(details: FreeTrialDetails) -> Result<String> {
    let response = unsafe {
        external::request_free_trial(
            CString::new(details.name)?.as_ptr(),
            CString::new(details.email)?.as_ptr(),
            CString::new(details.business_name)?.as_ptr(),
            CString::new(details.address1)?.as_ptr(),
            CString::new(details.address2)?.as_ptr(),
            CString::new(details.city)?.as_ptr(),
            CString::new(details.state)?.as_ptr(),
            CString::new(details.zip)?.as_ptr(),
            CString::new(details.country)?.as_ptr(),
            CString::new(details.phone)?.as_ptr(),
            CString::new(details.website)?.as_ptr(),
        )
    };
    if response == null_mut() {
        error!("Failed to request free trial");
        bail!("Failed to request free trial");
    } else {
        let response = unsafe { CString::from_raw(response) };
        let response = response.to_str()?;
        println!("Free Trial Status: {}", response);
        Ok(response.to_string())
    }
}

pub fn network_tree(timestamp: u64, tree: &[u8]) -> Result<()> {
    unsafe {
        if external::submit_network_tree(timestamp, tree.as_ptr(), tree.len()) != 0 {
            bail!("Failed to submit network tree");
        } else {
            Ok(())
        }
    }
}

pub fn shaped_devices(timestamp: u64, devices: &[u8]) -> Result<()> {
    unsafe {
        if external::submit_shaped_devices(timestamp, devices.as_ptr(), devices.len()) != 0 {
            bail!("Failed to submit shaped devices");
        } else {
            Ok(())
        }
    }
}

pub fn total_throughput(
    timestamp: u64,
    download_bytes: u64,
    upload_bytes: u64,
    shaped_download_bytes: u64,
    shaped_upload_bytes: u64,
    packets_down: u64,
    packets_up: u64,
    max_rtt: Option<f32>,
    min_rtt: Option<f32>,
    median_rtt: Option<f32>,
    tcp_retransmits_down: i32,
    tcp_retransmits_up: i32,
    cake_marks_down: i32,
    cake_marks_up: i32,
    cake_drops_down: i32,
    cake_drops_up: i32,
) -> Result<()> {
    unsafe {
        if external::submit_total_throughput(
            timestamp,
            download_bytes,
            upload_bytes,
            shaped_download_bytes,
            shaped_upload_bytes,
            packets_down,
            packets_up,
            max_rtt.is_some(),
            max_rtt.unwrap_or(0.0),
            min_rtt.is_some(),
            min_rtt.unwrap_or(0.0),
            median_rtt.is_some(),
            median_rtt.unwrap_or(0.0),
            tcp_retransmits_down,
            tcp_retransmits_up,
            cake_marks_down,
            cake_marks_up,
            cake_drops_down,
            cake_drops_up,
        ) != 0
        {
            bail!("Failed to submit total throughput");
        } else {
            Ok(())
        }
    }
}

pub fn circuit_throughput(data: &[shared_types::CircuitThroughput]) -> Result<()> {
    unsafe {
        if external::submit_circuit_throughput_batch(data.as_ptr(), data.len()) != 0 {
            bail!("Failed to submit circuit throughput");
        } else {
            Ok(())
        }
    }
}

pub fn circuit_retransmits(data: &[shared_types::CircuitRetransmits]) -> Result<()> {
    unsafe {
        if external::submit_circuit_retransmits_batch(data.as_ptr(), data.len()) != 0 {
            bail!("Failed to submit circuit retransmits");
        } else {
            Ok(())
        }
    }
}

pub fn circuit_rtt(data: &[shared_types::CircuitRtt]) -> Result<()> {
    unsafe {
        if external::submit_circuit_rtt_batch(data.as_ptr(), data.len()) != 0 {
            bail!("Failed to submit circuit rtt");
        } else {
            Ok(())
        }
    }
}

pub fn circuit_cake_drops(data: &[shared_types::CircuitCakeDrops]) -> Result<()> {
    unsafe {
        if external::submit_circuit_cake_drops_batch(data.as_ptr(), data.len()) != 0 {
            bail!("Failed to submit circuit cake drops");
        } else {
            Ok(())
        }
    }
}

pub fn circuit_cake_marks(data: &[shared_types::CircuitCakeMarks]) -> Result<()> {
    unsafe {
        if external::submit_circuit_cake_marks_batch(data.as_ptr(), data.len()) != 0 {
            bail!("Failed to submit circuit cake marks");
        } else {
            Ok(())
        }
    }
}

pub fn site_throughput(data: &[shared_types::SiteThroughput]) -> Result<()> {
    unsafe {
        if external::submit_site_throughput_batch(data.as_ptr(), data.len()) != 0 {
            bail!("Failed to submit site throughput");
        } else {
            Ok(())
        }
    }
}

pub fn site_retransmits(data: &[shared_types::SiteRetransmits]) -> Result<()> {
    unsafe {
        if external::submit_site_retransmits_batch(data.as_ptr(), data.len()) != 0 {
            bail!("Failed to submit site retransmits");
        } else {
            Ok(())
        }
    }
}

pub fn site_rtt(data: &[shared_types::SiteRtt]) -> Result<()> {
    unsafe {
        if external::submit_site_rtt_batch(data.as_ptr(), data.len()) != 0 {
            bail!("Failed to submit site rtt");
        } else {
            Ok(())
        }
    }
}

pub fn site_cake_drops(data: &[shared_types::SiteCakeDrops]) -> Result<()> {
    unsafe {
        if external::submit_site_cake_drops_batch(data.as_ptr(), data.len()) != 0 {
            bail!("Failed to submit site cake drops");
        } else {
            Ok(())
        }
    }
}

pub fn site_cake_marks(data: &[shared_types::SiteCakeMarks]) -> Result<()> {
    unsafe {
        if external::submit_site_cake_marks_batch(data.as_ptr(), data.len()) != 0 {
            bail!("Failed to submit site cake marks");
        } else {
            Ok(())
        }
    }
}

pub fn get_lts_license_status() -> (LtsStatus, i32) {
    unsafe {
        let remaining = external::get_lts_license_trial_remaining();
        let status = external::get_lts_license_status();
        (LtsStatus::from_i32(status), remaining)
    }
}

struct Lts2Config {
    has_remote_host: bool,
    remote_host: CString,
    license_key: CString,
    node_id: CString,
    node_name: CString,
}

fn get_config() -> anyhow::Result<Lts2Config> {
    if let Ok(config) = load_config() {
        let license_key = if let Some(ref key) = config.long_term_stats.license_key {
            key.to_string()
        } else {
            String::new()
        };

        let remote_host = if let Some(ref host) = config.long_term_stats.lts_url {
            host.to_string()
        } else {
            String::new()
        };

        let remote_host = std::ffi::CString::new(remote_host).unwrap();
        let license_key = std::ffi::CString::new(license_key).unwrap();
        let node_id = std::ffi::CString::new(config.node_id).unwrap();
        let node_name = std::ffi::CString::new(config.node_name).unwrap();

        Ok(Lts2Config {
            has_remote_host: config.long_term_stats.lts_url.is_some(),
            remote_host,
            license_key,
            node_id,
            node_name,
        })
    } else {
        bail!("Failed to load config");
    }
}