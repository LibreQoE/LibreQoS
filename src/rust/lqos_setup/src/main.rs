mod bandwidth;
mod bridge_mode;
mod config_builder;
mod interfaces;
mod ip_range;
mod preflight;
mod webusers;

use std::path::Path;
use std::{env, fmt::Write as _};

use bandwidth::bandwidth_view;
use config_builder::CURRENT_CONFIG;
use cursive::{
    Rect, Vec2, View,
    view::{Margins, Nameable, Resizable},
    views::{Checkbox, Dialog, EditView, FixedLayout, Layer, LinearLayout, OnLayoutView, TextView},
};

const VERSION: &str = include_str!("../../../VERSION_STRING");
const DEFAULT_NETWORK_JSON: &str = include_str!("../../../network.example.json");
const DEFAULT_SHAPED_DEVICES: &str = include_str!("../../../ShapedDevices.example.csv");
const SKIP_IF_READY_FLAG: &str = "--skip-if-ready";

fn config_exists() -> bool {
    let config_path = Path::new("/etc/lqos.conf");
    config_path.exists()
}

fn can_load_config() -> bool {
    lqos_config::load_config().is_ok()
}

fn network_json_exists() -> bool {
    let cfg = lqos_config::load_config().unwrap_or_default();
    let path = Path::new(&cfg.lqos_directory).join("network.json");
    path.exists()
}

fn shaped_devices_exists() -> bool {
    let cfg = lqos_config::load_config().unwrap_or_default();
    let path = Path::new(&cfg.lqos_directory).join("ShapedDevices.csv");
    path.exists()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UpgradeReadiness {
    config_present: bool,
    config_loads: bool,
    interfaces_ready: bool,
    bandwidth_ready: bool,
    network_json_present: bool,
    shaped_devices_present: bool,
}

impl UpgradeReadiness {
    fn all_ready(&self) -> bool {
        self.config_present
            && self.config_loads
            && self.interfaces_ready
            && self.bandwidth_ready
            && self.network_json_present
            && self.shaped_devices_present
    }

    fn summary(&self) -> String {
        let mut message = String::from("lqos_setup --skip-if-ready checklist:");
        let checklist = [
            ("config present", self.config_present),
            ("config loads", self.config_loads),
            ("interfaces chosen", self.interfaces_ready),
            ("bandwidth configured", self.bandwidth_ready),
            ("network.json present", self.network_json_present),
            ("ShapedDevices.csv present", self.shaped_devices_present),
        ];
        for (label, ready) in checklist {
            let _ = write!(
                &mut message,
                "\n- {}: {}",
                label,
                if ready { "yes" } else { "no" }
            );
        }
        message
    }
}

fn upgrade_readiness_for_config_with(
    config_path_exists: bool,
    config: Option<&lqos_config::Config>,
    interface_ready: impl Fn(&str) -> bool,
) -> UpgradeReadiness {
    let Some(config) = config else {
        return UpgradeReadiness {
            config_present: config_path_exists,
            config_loads: false,
            interfaces_ready: false,
            bandwidth_ready: false,
            network_json_present: false,
            shaped_devices_present: false,
        };
    };

    let base = Path::new(&config.lqos_directory);
    let interfaces_ready = if let Some(bridge) = &config.bridge {
        !bridge.to_internet.trim().is_empty()
            && !bridge.to_network.trim().is_empty()
            && bridge.to_internet != bridge.to_network
            && interface_ready(&bridge.to_internet)
            && interface_ready(&bridge.to_network)
    } else if let Some(single) = &config.single_interface {
        !single.interface.trim().is_empty() && interface_ready(&single.interface)
    } else {
        false
    };

    UpgradeReadiness {
        config_present: config_path_exists,
        config_loads: true,
        interfaces_ready,
        bandwidth_ready: config.queues.downlink_bandwidth_mbps > 0
            && config.queues.uplink_bandwidth_mbps > 0,
        network_json_present: base.join("network.json").exists(),
        shaped_devices_present: base.join("ShapedDevices.csv").exists(),
    }
}

fn upgrade_readiness() -> UpgradeReadiness {
    let config_path_exists = config_exists();
    let loaded_config = lqos_config::load_config().ok();
    upgrade_readiness_for_config_with(config_path_exists, loaded_config.as_deref(), |interface| {
        interfaces::interface_supports_lqos(interface).is_ok()
    })
}

fn should_skip_interactive_setup() -> bool {
    env::args().skip(1).any(|arg| arg == SKIP_IF_READY_FLAG)
}

fn main() {
    if should_skip_interactive_setup() {
        let readiness = upgrade_readiness();
        println!("{}", readiness.summary());
        if readiness.all_ready() {
            println!("Existing LibreQoS install looks configured; skipping interactive setup.");
            return;
        }
        println!("Interactive setup still required.");
    }

    preflight::preflight();
    let mut ui = cursive::default();
    ui.add_global_callback('q', |s| s.quit());

    ui.screen_mut().add_transparent_layer(
        OnLayoutView::new(
            FixedLayout::new().child(
                Rect::from_point(Vec2::zero()),
                Layer::new(TextView::new("Press [Q] To Quit Without Saving").with_name("status"))
                    .full_width(),
            ),
            |layout, size| {
                // We could also keep the status bar at the top instead.
                layout.set_child_position(0, Rect::from_size((0, size.y - 1), (size.x, 1)));
                layout.layout(size);
            },
        )
        .full_screen(),
    );

    let (node_id, node_name) = {
        let config = lqos_config::load_config().unwrap_or_default();
        (config.node_id.clone(), config.node_name.clone())
    };

    ui.add_layer(
        Dialog::new()
            .title(format!("LQOS Setup - v{VERSION}"))
            .content(
                LinearLayout::vertical()
                    .child(TextView::new("Welcome to the LQOS Setup!"))
                    .child(
                        LinearLayout::horizontal()
                            .child(
                                Checkbox::new()
                                    .with_enabled(false)
                                    .with_checked(config_exists())
                                    .with_name("config_exists"),
                            )
                            .child(TextView::new(" - Configuration file exists?")),
                    )
                    .child(
                        LinearLayout::horizontal()
                            .child(
                                Checkbox::new()
                                    .with_enabled(false)
                                    .with_checked(can_load_config())
                                    .with_name("config_loads"),
                            )
                            .child(TextView::new(" - Configuration loads?")),
                    )
                    .child(
                        LinearLayout::horizontal()
                            .child(
                                Checkbox::new()
                                    .with_enabled(false)
                                    .with_checked(network_json_exists())
                                    .with_name("njs"),
                            )
                            .child(TextView::new(" - Network.json exists?")),
                    )
                    .child(
                        LinearLayout::horizontal()
                            .child(
                                Checkbox::new()
                                    .with_enabled(false)
                                    .with_checked(shaped_devices_exists())
                                    .with_name("sd"),
                            )
                            .child(TextView::new(" - ShapedDevices.csv exists?")),
                    )
                    .child(TextView::new(" "))
                    .child(
                        LinearLayout::horizontal()
                            .child(
                                LinearLayout::vertical()
                                    .child(TextView::new("Node Id  :"))
                                    .child(TextView::new("Node Name:")),
                            )
                            .child(
                                LinearLayout::vertical()
                                    .child(TextView::new(node_id))
                                    .child(
                                        EditView::new()
                                            .on_edit(|_s, content, _cursor| {
                                                let mut config = CURRENT_CONFIG.lock();
                                                config.node_name = content.to_string();
                                            })
                                            .content(node_name)
                                            .with_name("node_name"),
                                    ),
                            ),
                    ),
            )
            .padding(Margins::lrtb(1, 1, 1, 1))
            .button("Bridge Mode", bridge_mode::bridge_mode)
            .button("Interfaces", interfaces::interface_menu)
            .button("Bandwidth", bandwidth_view)
            .button("IP Range", ip_range::ranges)
            .button("Web Users", webusers::webusers_menu)
            .button("SAVE CONFIG", finalize),
    );
    ui.run();
}

fn finalize(ui: &mut cursive::Cursive) {
    // If we cannot load the config but a file exists, warn the user and
    // take a backup before proceeding to create a new config.
    let config_path = Path::new("/etc/lqos.conf");
    let load_result = lqos_config::load_config();
    if load_result.is_err() && config_path.exists() {
        let backup_path = "/etc/lqos.conf.setupbackup";
        let backup_result = std::fs::copy(config_path, backup_path);

        let msg = match backup_result {
            Ok(_) => format!(
                "An existing configuration file was found at /etc/lqos.conf,\n\
but it could not be read or parsed.\n\n\
A backup has been saved to: {}\n\n\
Press Continue to create a new configuration using defaults,\n\
or Cancel to exit and investigate.",
                backup_path
            ),
            Err(e) => format!(
                "An existing configuration file was found at /etc/lqos.conf,\n\
but it could not be read or parsed.\n\n\
Attempted to back it up, but failed: {:?}\n\n\
Press Continue to create a new configuration using defaults,\n\
or Cancel to exit and investigate.",
                e
            ),
        };

        ui.add_layer(
            Dialog::around(TextView::new(msg))
                .title("Existing Config Unreadable")
                .button("Continue", |s| {
                    s.pop_layer();
                    continue_finalize(s);
                })
                .button("Cancel", |s| {
                    s.pop_layer();
                }),
        );
        return;
    }

    // Otherwise, proceed to saving normally.
    continue_finalize(ui);
}

fn continue_finalize(ui: &mut cursive::Cursive) {
    let mut event_log = Vec::new();

    // Update/Create the config file.
    let mut config = if let Ok(config) = lqos_config::load_config() {
        event_log.push("Loaded existing configuration".to_string());
        (*config).clone()
    } else {
        // If the file exists but couldn't be read, ensure we also log that a
        // backup was attempted (final safeguard; may already have been done
        // before showing the warning dialog).
        if Path::new("/etc/lqos.conf").exists() {
            let backup_path = "/etc/lqos.conf.setupbackup";
            match std::fs::copy("/etc/lqos.conf", backup_path) {
                Ok(_) => event_log.push(format!(
                    "Existing /etc/lqos.conf could not be loaded. Backup saved to {}.",
                    backup_path
                )),
                Err(e) => event_log.push(format!(
                    "Existing /etc/lqos.conf could not be loaded. Backup attempt failed: {:?}.",
                    e
                )),
            }
        }
        event_log.push("Creating new configuration".to_string());
        lqos_config::Config::default()
    };

    let new_config = CURRENT_CONFIG.lock();
    config.node_name = new_config.node_name.clone();
    config.queues.downlink_bandwidth_mbps = new_config.mbps_to_internet;
    config.queues.uplink_bandwidth_mbps = new_config.mbps_to_network;
    config.queues.generated_pn_download_mbps = new_config.mbps_to_internet;
    config.queues.generated_pn_upload_mbps = new_config.mbps_to_network;
    match new_config.bridge_mode {
        config_builder::BridgeMode::Linux => {
            config.single_interface = None;
            config.bridge = Some(lqos_config::BridgeConfig {
                use_xdp_bridge: false,
                to_internet: new_config.to_internet.clone(),
                to_network: new_config.to_network.clone(),
            });
        }
        config_builder::BridgeMode::XDP => {
            config.single_interface = None;
            config.bridge = Some(lqos_config::BridgeConfig {
                use_xdp_bridge: true,
                to_internet: new_config.to_internet.clone(),
                to_network: new_config.to_network.clone(),
            });
        }
        config_builder::BridgeMode::Single => {
            config.single_interface = Some(lqos_config::SingleInterfaceConfig {
                interface: new_config.to_internet.clone(),
                internet_vlan: new_config.internet_vlan,
                network_vlan: new_config.network_vlan,
            });
            config.bridge = None;
        }
    }
    config.ip_ranges.allow_subnets = new_config.allow_subnets.clone();
    if let Err(e) = lqos_config::update_config(&config) {
        event_log.push(format!("ERROR: Unable to write configuration: {e:?}"));
        let msg = format!("ERROR: Unable to write configuration: {e:?}");
        ui.add_layer(
            Dialog::around(TextView::new(msg))
                .title("Error")
                .button("OK", |s| {
                    s.pop_layer();
                }),
        );
        return;
    }
    event_log.push("Configuration updated".to_string());

    let state_root = config.resolved_state_directory();
    for category in ["topology", "shaping", "stats", "cache", "debug", "quarantine"] {
        std::fs::create_dir_all(state_root.join(category)).expect("Unable to create state directory");
    }

    // Does network.json exist?
    if !network_json_exists() {
        let path = Path::new(&config.lqos_directory).join("network.json");
        std::fs::write(path, DEFAULT_NETWORK_JSON).expect("Unable to write file");
        event_log.push("Network.json created.".to_string());
    } else {
        event_log.push("Network.json already exists - not updated.".to_string());
    }

    // Does ShapedDevices.csv exist?
    if !shaped_devices_exists() {
        let path = Path::new(&config.lqos_directory).join("ShapedDevices.csv");
        std::fs::write(path, DEFAULT_SHAPED_DEVICES).expect("Unable to write file");
        event_log.push("ShapedDevices.csv created.".to_string());
    } else {
        event_log.push("ShapedDevices.csv already exists - not updated.".to_string());
    }

    // Display final report
    use cursive::views::{Dialog, LinearLayout, TextView};

    let report = cursive::With::with(LinearLayout::vertical(), |layout| {
        for line in &event_log {
            layout.add_child(TextView::new(line));
        }
    });

    ui.add_layer(
        Dialog::around(report)
            .title("Setup Complete")
            .button("OK", |ui| ui.quit()),
    );
}

#[cfg(test)]
mod test {
    use super::upgrade_readiness_for_config_with;

    fn configured_bridge_config(lqos_directory: String) -> lqos_config::Config {
        let mut config = lqos_config::Config {
            lqos_directory,
            state_directory: None,
            bridge: Some(lqos_config::BridgeConfig {
                use_xdp_bridge: true,
                to_internet: "wan0".to_string(),
                to_network: "lan0".to_string(),
            }),
            single_interface: None,
            ..lqos_config::Config::default()
        };
        config.queues.downlink_bandwidth_mbps = 1000;
        config.queues.uplink_bandwidth_mbps = 1000;
        config
    }

    #[test]
    fn upgrade_ready_when_existing_install_is_configured() {
        let temp_dir =
            std::env::temp_dir().join(format!("lqos-setup-upgrade-ready-{}", std::process::id()));
        std::fs::create_dir_all(&temp_dir).unwrap();
        std::fs::write(temp_dir.join("network.json"), "{}").unwrap();
        std::fs::write(temp_dir.join("ShapedDevices.csv"), "Circuit ID\n").unwrap();

        let config = configured_bridge_config(temp_dir.display().to_string());
        let readiness = upgrade_readiness_for_config_with(true, Some(&config), |_| true);
        assert!(readiness.all_ready());

        let _ = std::fs::remove_file(temp_dir.join("network.json"));
        let _ = std::fs::remove_file(temp_dir.join("ShapedDevices.csv"));
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn upgrade_not_ready_without_required_runtime_inputs() {
        let temp_dir = std::env::temp_dir().join(format!(
            "lqos-setup-upgrade-missing-files-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let config = configured_bridge_config(temp_dir.display().to_string());
        let readiness = upgrade_readiness_for_config_with(true, Some(&config), |_| true);
        assert!(!readiness.all_ready());
        assert!(!readiness.network_json_present);
        assert!(!readiness.shaped_devices_present);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn upgrade_not_ready_when_bridge_interfaces_are_not_distinct() {
        let temp_dir = std::env::temp_dir().join(format!(
            "lqos-setup-upgrade-same-iface-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();
        std::fs::write(temp_dir.join("network.json"), "{}").unwrap();
        std::fs::write(temp_dir.join("ShapedDevices.csv"), "Circuit ID\n").unwrap();

        let mut config = configured_bridge_config(temp_dir.display().to_string());
        if let Some(bridge) = config.bridge.as_mut() {
            bridge.to_network = bridge.to_internet.clone();
        }
        let readiness = upgrade_readiness_for_config_with(true, Some(&config), |_| true);
        assert!(!readiness.all_ready());
        assert!(!readiness.interfaces_ready);

        let _ = std::fs::remove_file(temp_dir.join("network.json"));
        let _ = std::fs::remove_file(temp_dir.join("ShapedDevices.csv"));
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
