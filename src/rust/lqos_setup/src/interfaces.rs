use std::{path::Path, process::Command};

use cursive::{
    Cursive,
    view::Resizable,
    views::{Dialog, LinearLayout, RadioButton, RadioGroup, TextView},
};

use crate::config_builder::{BridgeMode, CURRENT_CONFIG};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InterfaceOption {
    pub(crate) name: String,
    pub(crate) label: String,
    pub(crate) bridge_eligible: bool,
    pub(crate) compatibility_shim_eligible: bool,
    pub(crate) single_interface_eligible: bool,
}

impl InterfaceOption {
    pub(crate) fn is_eligible_for(&self, mode: BridgeMode) -> bool {
        match mode {
            BridgeMode::Linux | BridgeMode::XDP => self.bridge_eligible,
            BridgeMode::CompatibilityShim => self.compatibility_shim_eligible,
            BridgeMode::Single => self.single_interface_eligible,
        }
    }
}

pub(crate) fn get_interface_options() -> Vec<InterfaceOption> {
    let inspection = lqos_netplan_helper::inspect_network_mode(&lqos_config::Config::default());
    inspection
        .interface_candidates
        .into_iter()
        .filter(|candidate| {
            candidate.bridge_eligible
                || candidate.compatibility_shim_eligible
                || candidate.single_interface_eligible
        })
        .map(|candidate| InterfaceOption {
            label: interface_label(&candidate.name),
            name: candidate.name,
            bridge_eligible: candidate.bridge_eligible,
            compatibility_shim_eligible: candidate.compatibility_shim_eligible,
            single_interface_eligible: candidate.single_interface_eligible,
        })
        .collect()
}

fn eligible_interface_options(mode: BridgeMode) -> Vec<InterfaceOption> {
    get_interface_options()
        .into_iter()
        .filter(|interface| interface.is_eligible_for(mode))
        .collect()
}

pub(crate) fn validate_mode_interfaces(
    mode: BridgeMode,
    to_internet: &str,
    to_network: &str,
) -> anyhow::Result<()> {
    validate_mode_interfaces_with_options(&get_interface_options(), mode, to_internet, to_network)
}

pub(crate) fn validate_mode_interfaces_with_options(
    options: &[InterfaceOption],
    mode: BridgeMode,
    to_internet: &str,
    to_network: &str,
) -> anyhow::Result<()> {
    if mode == BridgeMode::Single {
        if to_internet.is_empty() {
            anyhow::bail!("A shaping interface is required.");
        }
        return validate_interface_selection(options, to_internet, mode, "Shaping");
    }
    if to_internet.is_empty() {
        anyhow::bail!("An internet-facing interface is required.");
    }
    if to_network.is_empty() {
        anyhow::bail!("A network-facing interface is required.");
    }
    if to_internet == to_network {
        anyhow::bail!("Internet and network interfaces must be different.");
    }
    validate_interface_selection(options, to_internet, mode, "Internet-facing")?;
    validate_interface_selection(options, to_network, mode, "Network-facing")
}

fn validate_interface_selection(
    options: &[InterfaceOption],
    interface: &str,
    mode: BridgeMode,
    role: &str,
) -> anyhow::Result<()> {
    if options
        .iter()
        .any(|option| option.name == interface && option.is_eligible_for(mode))
    {
        Ok(())
    } else {
        anyhow::bail!("{role} interface {interface} is not eligible for the selected mode.")
    }
}

fn interface_label(interface: &str) -> String {
    let mut parts = vec![interface.to_string()];
    if let Some(speed) = interface_speed_label(interface) {
        parts.push(speed);
    }
    if let Some(kind) = interface_kind_label(interface) {
        parts.push(kind);
    }
    parts.join(" - ")
}

fn interface_speed_label(interface: &str) -> Option<String> {
    let path = Path::new("/sys/class/net").join(interface).join("speed");
    let raw = std::fs::read_to_string(path).ok()?;
    let speed = raw.trim().parse::<u64>().ok()?;
    if speed == 0 || speed == u32::MAX as u64 || speed == u64::MAX {
        return None;
    }

    let label = match speed {
        1000 => "1G".to_string(),
        2500 => "2.5G".to_string(),
        5000 => "5G".to_string(),
        10000 => "10G".to_string(),
        25000 => "25G".to_string(),
        40000 => "40G".to_string(),
        50000 => "50G".to_string(),
        100000 => "100G".to_string(),
        value if value >= 1000 && value % 1000 == 0 => format!("{}G", value / 1000),
        value => format!("{value}M"),
    };
    Some(label)
}

fn interface_kind_label(interface: &str) -> Option<String> {
    if let Some(port) = interface_port_label(interface) {
        return Some(port);
    }

    let driver_path = Path::new("/sys/class/net")
        .join(interface)
        .join("device/driver");
    if let Ok(target) = std::fs::read_link(driver_path)
        && let Some(driver) = target.file_name().and_then(|name| name.to_str())
    {
        return Some(driver.to_string());
    }

    let device_path = Path::new("/sys/class/net").join(interface).join("device");
    if !device_path.exists() {
        return Some("virtual".to_string());
    }

    None
}

fn interface_port_label(interface: &str) -> Option<String> {
    let output = Command::new("ethtool").arg(interface).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.trim() != "Port" {
            continue;
        }
        return normalize_port_label(value.trim());
    }
    None
}

fn normalize_port_label(port: &str) -> Option<String> {
    match port {
        "FIBRE" => Some("fiber".to_string()),
        "Twisted Pair" => Some("RJ45".to_string()),
        "Direct Attach Copper" => Some("DAC".to_string()),
        "Backplane" => Some("backplane".to_string()),
        "AUI" | "MII" | "BNC" => Some(port.to_ascii_lowercase()),
        "Other" | "Unknown" | "Internal" => None,
        _ if port.is_empty() => None,
        _ => Some(port.to_string()),
    }
}

fn build_interface_list(
    interfaces: &[InterfaceOption],
    group: &mut RadioGroup<String>,
    active: String,
) -> Vec<RadioButton<String>> {
    let mut buttons = Vec::new();
    for iface in interfaces {
        if iface.name != active {
            buttons.push(group.button(iface.name.clone(), iface.label.clone()));
        } else {
            let mut button = group.button(iface.name.clone(), iface.label.clone());
            button.select();
            buttons.push(button);
        }
    }
    buttons
}

fn build_layout() -> LinearLayout {
    let bridge_mode = CURRENT_CONFIG.lock().bridge_mode;
    match bridge_mode {
        BridgeMode::Linux | BridgeMode::XDP | BridgeMode::CompatibilityShim => {
            let interfaces = eligible_interface_options(bridge_mode);

            // If the configuration has empty interface fields, set them to the first available interface
            {
                let mut config = CURRENT_CONFIG.lock();
                if config.to_internet.is_empty() && !interfaces.is_empty() {
                    config.to_internet = interfaces[0].name.clone();
                }
                if config.to_network.is_empty() && !interfaces.is_empty() {
                    config.to_network = interfaces[0].name.clone();
                }
            }

            // Build up the Internet interface selection list
            let mut internet_group = RadioGroup::new().on_change(|_s, iface: &String| {
                let mut config = CURRENT_CONFIG.lock();
                config.to_internet = iface.to_string();
            });
            let internet_buttons = build_interface_list(
                &interfaces,
                &mut internet_group,
                CURRENT_CONFIG.lock().to_internet.clone(),
            );
            let mut internet_layout = LinearLayout::vertical();
            internet_layout.add_child(TextView::new("To Internet:"));
            for button in internet_buttons {
                internet_layout.add_child(button);
            }

            // Build up the Network interface selection list
            let mut network_group = RadioGroup::new().on_change(|_s, iface: &String| {
                let mut config = CURRENT_CONFIG.lock();
                config.to_network = iface.to_string();
            });
            let network_buttons = build_interface_list(
                &interfaces,
                &mut network_group,
                CURRENT_CONFIG.lock().to_network.clone(),
            );
            let mut network_layout = LinearLayout::vertical();
            network_layout.add_child(TextView::new("To Network:"));
            for button in network_buttons {
                network_layout.add_child(button);
            }

            LinearLayout::horizontal()
                // Left panel: To Internet
                .child(internet_layout)
                // Spacer between columns
                .child(TextView::new(" "))
                // Right panel: To Network
                .child(network_layout)
        }
        BridgeMode::Single => {
            let interfaces = eligible_interface_options(bridge_mode);

            // If the configuration has empty interface field, set it to the first available interface
            {
                let mut config = CURRENT_CONFIG.lock();
                if config.to_internet.is_empty() && !interfaces.is_empty() {
                    config.to_internet = interfaces[0].name.clone();
                }
            }

            let mut internet_group = RadioGroup::new().on_change(|_s, iface: &String| {
                let mut config = CURRENT_CONFIG.lock();
                config.to_internet = iface.to_string();
            });
            let internet_buttons = build_interface_list(
                &interfaces,
                &mut internet_group,
                CURRENT_CONFIG.lock().to_internet.clone(),
            );
            let mut internet_layout = LinearLayout::vertical();
            internet_layout.add_child(TextView::new("To Internet:"));
            for button in internet_buttons {
                internet_layout.add_child(button);
            }

            let (internet_vlan, network_vlan) = {
                let config = CURRENT_CONFIG.lock();
                (config.internet_vlan, config.network_vlan)
            };
            let vlan_layout = LinearLayout::vertical()
                .child(TextView::new("Internet VLAN:"))
                .child(
                    cursive::views::EditView::new()
                        .content(internet_vlan.to_string())
                        .on_edit(|s, content, _cursor| {
                            if content.is_empty() {
                                return;
                            }
                            if let Ok(vlan) = content.parse::<u32>() {
                                let mut config = CURRENT_CONFIG.lock();
                                config.internet_vlan = vlan;
                            } else {
                                s.add_layer(Dialog::info("Invalid VLAN number"));
                            }
                        })
                        .fixed_width(15),
                )
                .child(TextView::new("Network VLAN:"))
                .child(
                    cursive::views::EditView::new()
                        .content(network_vlan.to_string())
                        .on_edit(|s, content, _cursor| {
                            if content.is_empty() {
                                return;
                            }
                            if let Ok(vlan) = content.parse::<u32>() {
                                let mut config = CURRENT_CONFIG.lock();
                                config.network_vlan = vlan;
                            } else {
                                s.add_layer(Dialog::info("Invalid VLAN number"));
                            }
                        })
                        .fixed_width(15),
                );

            LinearLayout::horizontal()
                // Left panel: Single Interface
                .child(internet_layout)
                // Spacer between columns
                .child(TextView::new(" "))
                // Right panel: VLAN selection
                .child(vlan_layout)
        }
    }
}

pub fn interface_menu(s: &mut Cursive) {
    s.add_layer(
        Dialog::around(build_layout())
            .title("Select Interfaces")
            .button("OK", |s| {
                s.pop_layer();
                crate::preview_selected_network_mode(s);
            })
            .full_screen(),
    );
}

#[cfg(test)]
mod tests {
    use super::{InterfaceOption, validate_mode_interfaces_with_options};
    use crate::config_builder::BridgeMode;

    #[test]
    fn compatibility_shim_uses_its_relaxed_interface_eligibility() {
        let bond = InterfaceOption {
            name: "bond0".to_string(),
            label: "bond0".to_string(),
            bridge_eligible: false,
            compatibility_shim_eligible: true,
            single_interface_eligible: false,
        };

        assert!(bond.is_eligible_for(BridgeMode::CompatibilityShim));
        assert!(!bond.is_eligible_for(BridgeMode::Linux));
        assert!(!bond.is_eligible_for(BridgeMode::XDP));
        assert!(!bond.is_eligible_for(BridgeMode::Single));
    }

    #[test]
    fn compatibility_shim_rejects_stale_ineligible_selection() {
        let options = [InterfaceOption {
            name: "bond0".to_string(),
            label: "bond0".to_string(),
            bridge_eligible: false,
            compatibility_shim_eligible: true,
            single_interface_eligible: false,
        }];

        let error = validate_mode_interfaces_with_options(
            &options,
            BridgeMode::CompatibilityShim,
            "bond0",
            "stale0",
        )
        .expect_err("reject stale interface");

        assert!(
            error
                .to_string()
                .contains("Network-facing interface stale0")
        );
    }
}
