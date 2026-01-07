mod bandwidth;
mod bridge_mode;
mod config_builder;
mod interfaces;
mod ip_range;
mod preflight;
mod webusers;

use std::path::Path;

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

fn main() {
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
            .title(&format!("LQOS Setup - v{VERSION}"))
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
