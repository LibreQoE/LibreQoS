use std::{path::Path, process::Command};

use cursive::{
    Cursive,
    view::Resizable,
    views::{Dialog, LinearLayout, RadioButton, RadioGroup, TextView},
};

use crate::config_builder::{BridgeMode, CURRENT_CONFIG};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterfaceOption {
    pub name: String,
    pub label: String,
    pub role: InterfaceRole,
}

impl InterfaceOption {
    pub(crate) fn supports_mode(&self, mode: BridgeMode) -> bool {
        self.role.supports_mode(mode)
    }

    pub(crate) fn is_bond(&self) -> bool {
        matches!(self.role, InterfaceRole::BondMaster { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum InterfaceRole {
    Missing,
    Physical,
    BondMaster { native_xdp_issue: Option<String> },
    BondMember { master: String },
}

impl InterfaceRole {
    fn supports_mode(&self, mode: BridgeMode) -> bool {
        match self {
            Self::Missing => false,
            Self::Physical => true,
            Self::BondMaster { native_xdp_issue } => {
                mode == BridgeMode::XDP && native_xdp_issue.is_none()
            }
            Self::BondMember { .. } => false,
        }
    }

    fn ensure_mode(&self, interface: &str, mode: BridgeMode) -> anyhow::Result<()> {
        match self {
            Self::Missing => Err(anyhow::anyhow!(
                "Interface {interface} is not present on this system."
            )),
            Self::Physical => Ok(()),
            Self::BondMaster { .. } if mode != BridgeMode::XDP => Err(anyhow::anyhow!(
                "Bond master {interface} is supported only in XDP bridge mode."
            )),
            Self::BondMaster {
                native_xdp_issue: Some(issue),
            } => Err(anyhow::anyhow!("Bond master {interface}: {issue}")),
            Self::BondMaster {
                native_xdp_issue: None,
            } => Ok(()),
            Self::BondMember { master } => Err(anyhow::anyhow!(
                "Interface {interface} is a member of bond {master}; select the bond master instead."
            )),
        }
    }
}

pub fn get_interfaces() -> anyhow::Result<Vec<String>> {
    Ok(get_interface_options()?
        .into_iter()
        .map(|iface| iface.name)
        .collect())
}

pub fn get_interface_options() -> anyhow::Result<Vec<InterfaceOption>> {
    let mut interfaces = nix::ifaddrs::getifaddrs()?
        .filter(|iface| interface_supports_lqos(&iface.interface_name).is_ok())
        .map(|iface| iface.interface_name)
        .collect::<Vec<_>>();
    interfaces.sort();
    interfaces.dedup();
    Ok(interfaces
        .into_iter()
        .map(|name| {
            let role = interface_role(&name);
            InterfaceOption {
                label: interface_label(&name, &role),
                name,
                role,
            }
        })
        .collect())
}

pub(crate) fn ensure_interface_supports_mode(
    interface: &str,
    mode: BridgeMode,
) -> anyhow::Result<()> {
    interface_role(interface).ensure_mode(interface, mode)
}

fn interface_role(interface: &str) -> InterfaceRole {
    interface_role_at(Path::new("/sys/class/net"), interface)
}

fn interface_role_at(sys_class_net: &Path, interface: &str) -> InterfaceRole {
    let interface_path = sys_class_net.join(interface);
    if !interface_path.exists() {
        return InterfaceRole::Missing;
    }
    if interface_path.join("bonding").is_dir() {
        return InterfaceRole::BondMaster {
            native_xdp_issue: bond_native_xdp_issue(&interface_path),
        };
    }

    let master_path = interface_path.join("master");
    if let Ok(target) = std::fs::read_link(master_path)
        && let Some(master) = target.file_name().and_then(|name| name.to_str())
        && sys_class_net.join(master).join("bonding").is_dir()
    {
        return InterfaceRole::BondMember {
            master: master.to_string(),
        };
    }

    InterfaceRole::Physical
}

fn bond_native_xdp_issue(interface_path: &Path) -> Option<String> {
    let bonding_path = interface_path.join("bonding");
    let mode = std::fs::read_to_string(bonding_path.join("mode"))
        .ok()
        .and_then(|value| value.split_whitespace().next().map(ToOwned::to_owned));
    let transmit_hash_policy = std::fs::read_to_string(bonding_path.join("xmit_hash_policy"))
        .ok()
        .and_then(|value| value.split_whitespace().next().map(ToOwned::to_owned));
    lqos_config::native_xdp_bond_issue(mode.as_deref(), transmit_hash_policy.as_deref())
}

pub(crate) fn interface_supports_lqos(interface: &str) -> anyhow::Result<()> {
    let path = format!("/sys/class/net/{interface}/queues/");
    let sys_path = Path::new(&path);
    if !sys_path.exists() {
        return Err(anyhow::anyhow!(
            "/sys/class/net/{interface}/queues/ does not exist. Does this card only support one queue (not supported)?"
        ));
    }

    let mut counts = (0, 0);
    let paths = std::fs::read_dir(sys_path)?;
    for path in paths {
        if let Ok(path) = &path
            && path.path().is_dir()
            && let Some(filename) = path.path().file_name()
            && let Some(filename) = filename.to_str()
        {
            if filename.starts_with("rx-") {
                counts.0 += 1;
            } else if filename.starts_with("tx-") {
                counts.1 += 1;
            }
        }
    }

    if counts.0 == 0 || counts.1 == 0 {
        return Err(anyhow::anyhow!(
            "Interface {} does not have both RX and TX queues.",
            interface
        ));
    }
    if counts.0 == 1 || counts.1 == 1 {
        return Err(anyhow::anyhow!(
            "Interface {} only has one RX or TX queue. This is not supported.",
            interface
        ));
    }

    Ok(())
}

fn interface_label(interface: &str, role: &InterfaceRole) -> String {
    let mut parts = vec![interface.to_string()];
    if let Some(speed) = interface_speed_label(interface) {
        parts.push(speed);
    }
    if let Some(kind) = interface_kind_label(interface, role) {
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

fn interface_kind_label(interface: &str, role: &InterfaceRole) -> Option<String> {
    if matches!(role, InterfaceRole::BondMaster { .. }) {
        return Some("bond".to_string());
    }

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
        BridgeMode::Linux | BridgeMode::XDP => {
            let interfaces = get_interface_options()
                .expect("Failed to get interfaces")
                .into_iter()
                .filter(|interface| interface.supports_mode(bridge_mode))
                .collect::<Vec<_>>();

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
            let interfaces = get_interface_options()
                .expect("Failed to get interfaces")
                .into_iter()
                .filter(|interface| interface.supports_mode(bridge_mode))
                .collect::<Vec<_>>();

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
    get_interfaces().expect("Failed to get interfaces");
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
    use super::{InterfaceOption, InterfaceRole, interface_role_at};
    use crate::config_builder::BridgeMode;
    use std::path::PathBuf;

    fn test_sys_class_net(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "libreqos-setup-interfaces-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create fake sysfs root");
        root
    }

    #[test]
    fn bond_masters_are_only_offered_for_xdp_bridge_mode() {
        let bond = InterfaceOption {
            name: "bond0".to_string(),
            label: "bond0 - bond".to_string(),
            role: InterfaceRole::BondMaster {
                native_xdp_issue: None,
            },
        };

        assert!(bond.supports_mode(BridgeMode::XDP));
        assert!(!bond.supports_mode(BridgeMode::Linux));
        assert!(!bond.supports_mode(BridgeMode::Single));
    }

    #[test]
    fn physical_interfaces_remain_available_in_all_modes() {
        let interface = InterfaceOption {
            name: "enp1s0".to_string(),
            label: "enp1s0 - 10G".to_string(),
            role: InterfaceRole::Physical,
        };

        assert!(interface.supports_mode(BridgeMode::XDP));
        assert!(interface.supports_mode(BridgeMode::Linux));
        assert!(interface.supports_mode(BridgeMode::Single));
    }

    #[test]
    fn bond_members_are_never_offered_as_shaping_interfaces() {
        let member = InterfaceOption {
            name: "enp1s0".to_string(),
            label: "enp1s0 - 10G".to_string(),
            role: InterfaceRole::BondMember {
                master: "bond0".to_string(),
            },
        };

        assert!(!member.supports_mode(BridgeMode::XDP));
        assert!(!member.supports_mode(BridgeMode::Linux));
        assert!(!member.supports_mode(BridgeMode::Single));
        let error = member
            .role
            .ensure_mode(&member.name, BridgeMode::XDP)
            .unwrap_err();
        assert!(error.to_string().contains("select the bond master"));
    }

    #[test]
    fn incompatible_bond_modes_are_rejected_for_xdp() {
        let bond = InterfaceRole::BondMaster {
            native_xdp_issue: Some(
                "bond mode balance-alb does not support native XDP.".to_string(),
            ),
        };

        let error = bond.ensure_mode("bond0", BridgeMode::XDP).unwrap_err();
        assert!(error.to_string().contains("balance-alb"));
    }

    #[test]
    fn missing_interfaces_are_rejected() {
        let root = test_sys_class_net("missing-interface");
        let role = interface_role_at(&root, "not-present");

        assert_eq!(role, InterfaceRole::Missing);
        assert!(!role.supports_mode(BridgeMode::XDP));
        assert!(
            role.ensure_mode("not-present", BridgeMode::XDP)
                .unwrap_err()
                .to_string()
                .contains("not present")
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn live_bond_mode_and_membership_are_read_from_sysfs() {
        let root = test_sys_class_net("bond-role");
        let bonding = root.join("bond0/bonding");
        std::fs::create_dir_all(&bonding).expect("create fake bond sysfs");
        std::fs::write(bonding.join("mode"), "802.3ad 4\n").expect("write bond mode");
        std::fs::write(bonding.join("xmit_hash_policy"), "layer3+4 1\n")
            .expect("write bond hash policy");
        std::fs::create_dir_all(root.join("enp1s0")).expect("create fake member sysfs");
        std::os::unix::fs::symlink(root.join("bond0"), root.join("enp1s0/master"))
            .expect("link fake bond master");

        assert_eq!(
            interface_role_at(&root, "bond0"),
            InterfaceRole::BondMaster {
                native_xdp_issue: None,
            }
        );
        assert_eq!(
            interface_role_at(&root, "enp1s0"),
            InterfaceRole::BondMember {
                master: "bond0".to_string(),
            }
        );

        std::fs::write(bonding.join("mode"), "balance-alb 6\n").expect("replace bond mode");
        let InterfaceRole::BondMaster {
            native_xdp_issue: Some(issue),
        } = interface_role_at(&root, "bond0")
        else {
            panic!("expected incompatible bond master");
        };
        assert!(issue.contains("balance-alb"));

        let _ = std::fs::remove_dir_all(root);
    }
}
