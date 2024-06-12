use std::ffi::CString;
use std::path::Path;

use anyhow::Error;
use log::error;
use serde::{Deserialize, Serialize};

use lqos_config::load_config;

use crate::console::{error, success};

#[derive(Debug, Serialize, Deserialize)]
pub struct SanityChecks {
    results: Vec<SanityCheck>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct SanityCheck {
    pub name: String,
    pub success: bool,
    pub comments: String,
}

pub fn run_sanity_checks() -> anyhow::Result<SanityChecks> {
    println!("Running Sanity Checks");
    let mut results = Vec::new();

    // Run the checks
    config_exists(&mut results);
    can_load_config(&mut results);
    interfaces_exist(&mut results);
    sanity_check_queues(&mut results);

    // Did any fail?
    let mut any_errors = false;
    for s in results.iter() {
        if s.success {
            success(&s.name);
        } else {
            error(&format!("{}: {}", s.name, s.comments));
            any_errors = true;
        }
    }

    if any_errors {
        error("ERRORS FOUND DURING SANITY CHECK");
    }

    Ok(SanityChecks { results })
}

fn config_exists(results: &mut Vec<SanityCheck>) {
    let path = Path::new("/etc/lqos.conf");
    let mut result = SanityCheck {
        name: "Config File Exists".to_string(),
        ..Default::default()
    };
    if path.exists() {
        result.success = true;
    } else {
        result.success = false;
        result.comments = "/etc/lqos.conf could not be opened".to_string();
    }

    results.push(result);
}

fn can_load_config(results: &mut Vec<SanityCheck>) {
    let mut result = SanityCheck {
        name: "Config File Can Be Loaded".to_string(),
        ..Default::default()
    };
    let cfg = load_config();
    if cfg.is_ok() {
        result.success = true;
    } else {
        result.success = false;
        result.comments = "Configuration file could not be loaded".to_string();
    }
    results.push(result);
}

pub fn interface_name_to_index(interface_name: &str) -> anyhow::Result<u32> {
    use nix::libc::if_nametoindex;
    let if_name = CString::new(interface_name)?;
    let index = unsafe { if_nametoindex(if_name.as_ptr()) };
    if index == 0 {
        Err(Error::msg(format!("Unknown interface: {interface_name}")))
    } else {
        Ok(index)
    }
}

fn interfaces_exist(results: &mut Vec<SanityCheck>) {
    if let Ok(cfg) = load_config() {
        if cfg.on_a_stick_mode() {
            if interface_name_to_index(&cfg.internet_interface()).is_ok() {
                results.push(SanityCheck {
                    name: "Single Interface Exists".to_string(),
                    success: true,
                    comments: "".to_string(),
                });
            } else {
                results.push(SanityCheck {
                    name: "Single Interface Exists".to_string(),
                    success: false,
                    comments: format!("Interface {} is listed in /etc/lqos.conf - but that interface does not appear to exist in the Linux interface map", cfg.internet_interface()),
                });
            }
        } else {
            if interface_name_to_index(&cfg.internet_interface()).is_ok() {
                results.push(SanityCheck {
                    name: "Internet Interface Exists".to_string(),
                    success: true,
                    comments: "".to_string(),
                });
            } else {
                results.push(SanityCheck {
                    name: "Internet Interface Exists".to_string(),
                    success: false,
                    comments: format!("Interface {} is listed in /etc/lqos.conf - but that interface does not appear to exist in the Linux interface map", cfg.internet_interface()),
                });
            }

            if interface_name_to_index(&cfg.isp_interface()).is_ok() {
                results.push(SanityCheck {
                    name: "ISP Facing Interface Exists".to_string(),
                    success: true,
                    comments: "".to_string(),
                });
            } else {
                results.push(SanityCheck {
                    name: "ISP Facing Interface Exists".to_string(),
                    success: false,
                    comments: format!("Interface {} is listed in /etc/lqos.conf - but that interface does not appear to exist in the Linux interface map", cfg.isp_interface()),
                });
            }
        }
    }
}

fn check_queues(interface: &str) -> (i32, i32) {
    let path = format!("/sys/class/net/{interface}/queues/");
    let sys_path = Path::new(&path);
    if !sys_path.exists() {
        return (0,0);
    }

    let mut counts = (0, 0);
    let paths = std::fs::read_dir(sys_path).unwrap();
    for path in paths {
        if let Ok(path) = &path {
            if path.path().is_dir() {
                if let Some(filename) = path.path().file_name() {
                    if let Some(filename) = filename.to_str() {
                        if filename.starts_with("rx-") {
                            counts.0 += 1;
                        } else if filename.starts_with("tx-") {
                            counts.1 += 1;
                        }
                    }
                }
            }
        }
    }

    counts
}

fn sanity_check_queues(results: &mut Vec<SanityCheck>) {
    if let Ok(cfg) = load_config() {
        if cfg.on_a_stick_mode() {
            let counts = check_queues(&cfg.internet_interface());
            if counts.0 > 1 && counts.1 > 1 {
                results.push(SanityCheck{
                    name: "Queue Check (Internet Interface)".to_string(),
                    success: true,
                    comments: "".to_string(),
                });
            } else {
                results.push(SanityCheck{
                    name: "Queue Check (Internet Interface)".to_string(),
                    success: false,
                    comments: format!("{} does not provide multiple RX and TX queues", cfg.internet_interface()),
                });
            }
        } else {
            let counts = check_queues(&cfg.internet_interface());
            if counts.0 > 1 && counts.1 > 1 {
                results.push(SanityCheck{
                    name: "Queue Check (Internet Interface)".to_string(),
                    success: true,
                    comments: "".to_string(),
                });
            } else {
                results.push(SanityCheck{
                    name: "Queue Check (Internet Interface)".to_string(),
                    success: false,
                    comments: format!("{} does not provide multiple RX and TX queues", cfg.internet_interface()),
                });
            }

            let counts = check_queues(&cfg.isp_interface());
            if counts.0 > 1 && counts.1 > 1 {
                results.push(SanityCheck{
                    name: "Queue Check (ISP Facing Interface)".to_string(),
                    success: true,
                    comments: "".to_string(),
                });
            } else {
                results.push(SanityCheck{
                    name: "Queue Check (ISP Facing Interface)".to_string(),
                    success: false,
                    comments: format!("{} does not provide multiple RX and TX queues", cfg.isp_interface()),
                });
            }
        }
    }
}