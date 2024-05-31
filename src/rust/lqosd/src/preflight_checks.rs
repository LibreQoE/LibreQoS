use std::path::Path;

use anyhow::Result;
use log::{error, info};
use lqos_sys::interface_name_to_index;

fn check_queues(interface: &str) -> Result<()> {
    let path = format!("/sys/class/net/{interface}/queues/");
    let sys_path = Path::new(&path);
    if !sys_path.exists() {
        error!("/sys/class/net/{interface}/queues/ does not exist. Does this card only support one queue (not supported)?");
        return Err(anyhow::anyhow!("/sys/class/net/{interface}/queues/ does not exist. Does this card only support one queue (not supported)?"));
    }

    let mut counts = (0, 0);
    let paths = std::fs::read_dir(sys_path)?;
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

    if counts.0 == 0 || counts.1 == 0 {
        error!("Interface ({}) does not have both RX and TX queues.", interface);
        return Err(anyhow::anyhow!("Interface {} does not have both RX and TX queues.", interface));
    }
    if counts.0 == 1 || counts.1 == 1 {
        error!("Interface ({}) only has one RX or TX queue. This is not supported.", interface);
        return Err(anyhow::anyhow!("Interface {} only has one RX or TX queue. This is not supported.", interface));
    }

    Ok(())
}

/// Runs a series of preflight checks to ensure that the configuration is sane
pub fn preflight_checks() -> Result<()> {
    info!("Sanity checking configuration...");

    // Are we able to load the configuration?
    let config = lqos_config::load_config().map_err(|_| {
        error!("Failed to load configuration file - /etc/lqos.conf");
        anyhow::anyhow!("Failed to load configuration file")
    })?;

    // Do the interfaces exist?
    if config.on_a_stick_mode() {
        interface_name_to_index(&config.internet_interface()).map_err(|_| {
            error!(
                "Interface ({}) does not exist.",
                config.internet_interface()
            );
            anyhow::anyhow!("Interface {} does not exist", config.internet_interface())
        })?;
        check_queues(&config.internet_interface())?;
    } else {
        interface_name_to_index(&config.internet_interface()).map_err(|_| {
            error!(
                "Interface ({}) does not exist.",
                config.internet_interface()
            );
            anyhow::anyhow!("Interface {} does not exist", config.internet_interface())
        })?;
        interface_name_to_index(&config.isp_interface()).map_err(|_| {
            error!("Interface ({}) does not exist.", config.isp_interface());
            anyhow::anyhow!("Interface {} does not exist", config.isp_interface())
        })?;
        check_queues(&config.internet_interface())?;
        check_queues(&config.isp_interface())?;
    }

    info!("Sanity checks passed");

    Ok(())
}
