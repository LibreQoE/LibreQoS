use crate::sanity_checks::SanityCheck;
use lqos_config::load_config;
use std::process::Command;

pub fn check_bridge(results: &mut Vec<SanityCheck>) {
    if let Ok(cfg) = load_config()
        && let Ok(interfaces) = get_interfaces_from_ip_link()
    {
        // Single-interface mode is bridge-free
        if cfg.single_interface.is_some() {
            results.push(SanityCheck {
                name: "Single Interface Mode: Bridges Ignored".to_string(),
                success: true,
                comments: "".to_string(),
            });
            return;
        }

        // K3-B: the XDP bridge is retired; check Linux bridge membership.
        if cfg.bridge.is_some() {
            for bridge_if in interfaces.iter().filter(|bridge_if| {
                bridge_if.link_type == "ether" && bridge_if.operational_state == "UP"
            }) {
                // We found a bridge. Check member interfaces to check that it does NOT include the
                // LibreQoS-managed interfaces.
                let in_bridge: Vec<&IpLinkInterface> = interfaces
                    .iter()
                    .filter(|member_if| {
                        if let Some(master) = &member_if.master {
                            master == &bridge_if.name
                        } else {
                            false
                        }
                    })
                    .filter(|member_if| {
                        member_if.name == cfg.internet_interface()
                            || member_if.name == cfg.isp_interface()
                    })
                    .collect();

                if in_bridge.len() == 2 {
                    results.push(SanityCheck {
                        name: format!("Linux Bridge Membership ({})", bridge_if.name),
                        success: false,
                        comments: format!(
                            "Bridge ({}) contains both the internet and ISP interfaces, which LibreQoS manages directly. This is not supported.",
                            bridge_if.name
                        ),
                    });
                } else {
                    results.push(SanityCheck {
                        name: format!("Bridge Membership Check ({})", bridge_if.name),
                        success: true,
                        comments: "".to_string(),
                    });
                }
            }
        }
    }
}

pub fn check_interface_status(results: &mut Vec<SanityCheck>) {
    if let Ok(cfg) = load_config()
        && let Ok(interfaces) = get_interfaces_from_ip_link()
    {
        if let Some(stick) = &cfg.single_interface {
            if let Some(iface) = interfaces.iter().find(|i| i.name == stick.interface) {
                results.push(SanityCheck {
                    name: format!(
                        "Interface {} in state {}",
                        iface.name, iface.operational_state
                    ),
                    success: true,
                    comments: "".to_string(),
                });
            }
        } else if let Some(bridge) = &cfg.bridge {
            if let Some(iface) = interfaces.iter().find(|i| i.name == bridge.to_internet) {
                results.push(SanityCheck {
                    name: format!(
                        "Interface {} in state {}",
                        iface.name, iface.operational_state
                    ),
                    success: true,
                    comments: "".to_string(),
                });
            }
            if let Some(iface) = interfaces.iter().find(|i| i.name == bridge.to_network) {
                results.push(SanityCheck {
                    name: format!(
                        "Interface {} in state {}",
                        iface.name, iface.operational_state
                    ),
                    success: true,
                    comments: "".to_string(),
                });
            }
        }
    }
}

#[derive(Debug)]
struct IpLinkInterface {
    pub name: String,
    pub operational_state: String,
    pub link_type: String,
    pub master: Option<String>,
}

fn get_interfaces_from_ip_link() -> anyhow::Result<Vec<IpLinkInterface>> {
    let output = Command::new("/sbin/ip").args(["-j", "link"]).output()?;
    let output = String::from_utf8(output.stdout)?;
    let output_json = serde_json::from_str::<serde_json::Value>(&output)?;

    let mut interfaces = Vec::new();
    for interface in output_json.as_array().unwrap() {
        let name = interface["ifname"].as_str().unwrap().to_string();
        let operstate = interface["operstate"].as_str().unwrap().to_string();
        let link_type = interface["link_type"].as_str().unwrap().to_string();
        let master = interface["master"].as_str().map(|s| s.to_string());

        interfaces.push(IpLinkInterface {
            name,
            operational_state: operstate,
            link_type,
            master,
        });
    }

    Ok(interfaces)
}
