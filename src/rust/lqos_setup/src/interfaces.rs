use std::path::Path;

use cursive::{
    Cursive,
    view::Resizable,
    views::{Dialog, LinearLayout, RadioButton, RadioGroup, TextView},
};

use crate::config_builder::{BridgeMode, CURRENT_CONFIG};

pub fn get_interfaces() -> anyhow::Result<Vec<String>> {
    let interfaces = nix::ifaddrs::getifaddrs()?
        .filter(|iface| check_queues(&iface.interface_name).is_ok())
        .map(|iface| iface.interface_name)
        .collect::<Vec<_>>();
    Ok(interfaces)
}

fn check_queues(interface: &str) -> anyhow::Result<()> {
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

fn build_interface_list(
    interfaces: &[String],
    group: &mut RadioGroup<String>,
    active: String,
) -> Vec<RadioButton<String>> {
    let mut buttons = Vec::new();
    for iface in interfaces {
        if *iface != active {
            buttons.push(group.button(iface.clone(), iface));
        } else {
            let mut button = group.button(iface.clone(), iface);
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
            let interfaces = get_interfaces().expect("Failed to get interfaces");

            // If the configuration has empty interface fields, set them to the first available interface
            {
                let mut config = CURRENT_CONFIG.lock();
                if config.to_internet.is_empty() && !interfaces.is_empty() {
                    config.to_internet = interfaces[0].clone();
                }
                if config.to_network.is_empty() && !interfaces.is_empty() {
                    config.to_network = interfaces[0].clone();
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
            let interfaces = get_interfaces().expect("Failed to get interfaces");

            // If the configuration has empty interface field, set it to the first available interface
            {
                let mut config = CURRENT_CONFIG.lock();
                if config.to_internet.is_empty() && !interfaces.is_empty() {
                    config.to_internet = interfaces[0].clone();
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
            })
            .full_screen(),
    );
}
