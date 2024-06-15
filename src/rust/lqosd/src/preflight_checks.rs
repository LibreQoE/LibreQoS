use std::{path::Path, process::Command};

use anyhow::Result;
use log::{error, info};
use lqos_config::Config;
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

#[derive(Debug)]
pub struct IpLinkInterface {
    pub name: String,
    pub index: u32,
    pub operstate: String,
    pub link_type: String,
    pub master: Option<String>,
}

pub fn get_interfaces_from_ip_link() -> Result<Vec<IpLinkInterface>> {
    let output = Command::new("/sbin/ip")
        .args(["-j", "link"])
        .output()?;
    let output = String::from_utf8(output.stdout)?;
    let output_json = serde_json::from_str::<serde_json::Value>(&output)?;

    let mut interfaces = Vec::new();
    for interface in output_json.as_array().unwrap() {
        let name = interface["ifname"].as_str().unwrap().to_string();
        let index = interface["ifindex"].as_u64().unwrap() as u32;
        let operstate = interface["operstate"].as_str().unwrap().to_string();
        let link_type = interface["link_type"].as_str().unwrap().to_string();
        let master = interface["master"].as_str().map(|s| s.to_string());

        interfaces.push(IpLinkInterface {
            name,
            index,
            operstate,
            link_type,
            master,
        });
    }

    Ok(interfaces)
}

fn check_interface_status(config: &Config, interfaces: &[IpLinkInterface]) -> Result<()> {
    if let Some(stick) = &config.single_interface {
        if let Some(iface) = interfaces.iter().find(|i| i.name == stick.interface) {
            info!("Interface {} is in status: {}", stick.interface, iface.operstate);
        }
    } else if let Some(bridge) = &config.bridge {
        if let Some(iface) = interfaces.iter().find(|i| i.name == bridge.to_internet) {
            info!("Interface {} is in status: {}", iface.name, iface.operstate);
        }
        if let Some(iface) = interfaces.iter().find(|i| i.name == bridge.to_network) {
            info!("Interface {} is in status: {}", iface.name, iface.operstate);
        }
    } else {
        error!("You MUST have either a single interface or a bridge defined in the configuration file.");
        anyhow::bail!("You MUST have either a single interface or a bridge defined in the configuration file.");
    }

    Ok(())
}

fn check_bridge_status(config: &Config, interfaces: &[IpLinkInterface]) -> Result<()> {
    // On a stick mode is bridge-free
    if config.on_a_stick_mode() {        
        return Ok(());
    }

    // Is the XDP bridge enabled?
    if let Some(bridge) = &config.bridge {
        if bridge.use_xdp_bridge {
            for bridge_if in interfaces
                .iter()
                .filter(|bridge_if| bridge_if.link_type == "ether" && bridge_if.operstate == "UP") 
            {
                // We found a bridge. Check member interfaces to check that it does NOT include any XDP
                // bridge members.
                let in_bridge: Vec<&IpLinkInterface> = interfaces
                    .iter()
                    .filter(|member_if| {
                        if let Some(master) = &member_if.master {
                            master == &bridge_if.name
                        } else {
                            false
                        }
                    })
                    .filter(|member_if| member_if.name == config.internet_interface() || member_if.name == config.isp_interface())
                    .collect();

                if in_bridge.len() == 2 {
                    error!("Bridge ({}) contains both the internet and ISP interfaces, and you have the xdp_bridge enabled. This is not supported.", bridge_if.name);
                    anyhow::bail!("Bridge {} contains both the internet and ISP interfaces. This is not supported.", bridge_if.name);                    
                }
            }
        }
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

    // Obtain the "IP link" output
    let interfaces = get_interfaces_from_ip_link()?;

    // Are the interfaces up?
    check_interface_status(&config, &interfaces)?;

    // Does the bridge system make sense?
    if check_bridge_status(&config, &interfaces).is_err() {
        log::warn!("Disabling XDP bridge");
        lqos_config::disable_xdp_bridge()?;
        log::warn!("XDP bridge disabled in ACTIVE config. Please fix the configuration file.");
    };

    info!("Sanity checks passed");

    Ok(())
}
