use std::ffi::CString;
use anyhow::Error;
use lqos_config::load_config;
use crate::sanity_checks::SanityCheck;

fn interface_name_to_index(interface_name: &str) -> anyhow::Result<u32> {
    use nix::libc::if_nametoindex;
    let if_name = CString::new(interface_name)?;
    let index = unsafe { if_nametoindex(if_name.as_ptr()) };
    if index == 0 {
        Err(Error::msg(format!("Unknown interface: {interface_name}")))
    } else {
        Ok(index)
    }
}

pub fn interfaces_exist(results: &mut Vec<SanityCheck>) {
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