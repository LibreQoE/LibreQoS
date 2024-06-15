use std::path::Path;
use lqos_config::load_config;
use crate::sanity_checks::SanityCheck;

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

pub fn sanity_check_queues(results: &mut Vec<SanityCheck>) {
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