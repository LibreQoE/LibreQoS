mod bandwidth;
mod bridge_mode;
mod config_builder;
mod interfaces;
mod ip_range;
mod preflight;
mod service_handoff;
mod setup_actions;
mod web;
mod webusers;

use clap::{Parser, Subcommand};
use lqos_setup::{bootstrap, hotfix};
use std::path::Path;

use bandwidth::bandwidth_view;
use config_builder::CURRENT_CONFIG;
use cursive::{
    Rect, Vec2, View,
    view::{Margins, Nameable, Resizable, Scrollable},
    views::{Checkbox, Dialog, EditView, FixedLayout, Layer, LinearLayout, OnLayoutView, TextView},
};
use lqos_netplan_helper::transaction::{HelperPaths, inspect_with_paths};

const VERSION: &str = include_str!("../../../VERSION_STRING");
#[derive(Parser)]
#[command(about = "LibreQoS first-run setup")]
struct Args {
    #[arg(long)]
    skip_if_ready: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Launch the interactive Cursive setup UI
    Tui,
    /// Launch the setup-only web server
    Web,
    /// Print the current setup status
    Status,
    /// Print the current Ubuntu 24.04 systemd hotfix status
    HotfixStatus,
    /// Install the Ubuntu 24.04 systemd hotfix without prompting for reboot
    InstallHotfix,
    /// Exit 0 when runtime services should start, or 1 when setup is still required
    IsReady,
    /// Internal helper: stop setup and activate runtime services now
    ActivateRuntime,
    /// Internal helper: stop runtime and activate the first-run setup service now
    ActivateSetup,
    /// Create or refresh and print the current tokenized setup link(s)
    PrintLink,
}

fn config_exists() -> bool {
    let config_path = Path::new("/etc/lqos.conf");
    config_path.exists()
}

fn can_load_config() -> bool {
    lqos_config::load_config().is_ok()
}

fn run_tui(skip_if_ready: bool) {
    if skip_if_ready {
        match bootstrap::runtime_services_should_start() {
            Ok(true) => {
                println!("Existing LibreQoS install looks configured; skipping interactive setup.");
                return;
            }
            Ok(false) => {
                println!("Interactive setup still required.");
            }
            Err(err) => {
                println!("Unable to determine setup readiness: {err}");
                println!("Interactive setup still required.");
            }
        }
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
                                    .with_checked(bootstrap::first_admin_exists())
                                    .with_name("admin_exists"),
                            )
                            .child(TextView::new(" - Admin user exists?")),
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
            .button("Systemd Hotfix", show_hotfix_dialog)
            .button("Web Users", webusers::webusers_menu)
            .button("SAVE CONFIG", finalize),
    );
    ui.run();
}

fn main() {
    let args = Args::parse();

    match args.command {
        Some(Command::Status) => match bootstrap::render_status_report() {
            Ok(report) => println!("{report}"),
            Err(err) => {
                eprintln!("Unable to render setup status: {err:#}");
                std::process::exit(1);
            }
        },
        Some(Command::HotfixStatus) => match hotfix::status() {
            Ok(status) => println!("{}", status.detail),
            Err(err) => {
                eprintln!("Unable to determine hotfix status: {err:#}");
                std::process::exit(1);
            }
        },
        Some(Command::InstallHotfix) => match hotfix::install() {
            Ok(result) => {
                println!("{}", result.summary);
                println!();
                println!("{}", result.detail);
            }
            Err(err) => {
                eprintln!("Unable to install hotfix: {err:#}");
                std::process::exit(1);
            }
        },
        Some(Command::IsReady) => match bootstrap::runtime_services_should_start() {
            Ok(true) => {}
            Ok(false) => std::process::exit(1),
            Err(err) => {
                eprintln!("Unable to determine setup readiness: {err:#}");
                std::process::exit(1);
            }
        },
        Some(Command::ActivateRuntime) => match service_handoff::activate_runtime_services() {
            Ok(message) => println!("{message}"),
            Err(err) => {
                eprintln!("Unable to activate runtime services: {err:#}");
                std::process::exit(1);
            }
        },
        Some(Command::ActivateSetup) => match service_handoff::activate_setup_service() {
            Ok(message) => println!("{message}"),
            Err(err) => {
                eprintln!("Unable to activate setup service: {err:#}");
                std::process::exit(1);
            }
        },
        Some(Command::Web) => {
            if let Err(err) = web::run() {
                eprintln!("Unable to run setup web server: {err:#}");
                std::process::exit(1);
            }
        }
        Some(Command::PrintLink) => match bootstrap::current_setup_urls() {
            Ok(urls) => {
                if urls.is_empty() {
                    println!("Setup is already complete. No active setup link.");
                } else {
                    println!();
                    println!("============================================================");
                    println!("LibreQoS first-run setup is waiting for you.");
                    println!(
                        "Click one of the LibreQoS Setup URLs below, or copy it into a web browser on another machine on the same network."
                    );
                    println!();
                    for url in urls {
                        println!("  {url}");
                    }
                    println!("============================================================");
                    println!();
                }
            }
            Err(err) => {
                eprintln!("Unable to print setup link: {err:#}");
                std::process::exit(1);
            }
        },
        Some(Command::Tui) | None => run_tui(args.skip_if_ready),
    }
}

pub(crate) fn preview_selected_network_mode(s: &mut cursive::Cursive) {
    let existing = lqos_config::load_config().ok().map(|cfg| (*cfg).clone());
    let candidate = setup_actions::build_candidate_config(existing);
    let inspection = inspect_with_paths(&HelperPaths::default(), &candidate);
    s.add_layer(
        Dialog::around(TextView::new(setup_actions::inspection_report(&inspection)).scrollable())
            .title("Detected Netplan State")
            .button("OK", |s| {
                s.pop_layer();
            })
            .full_screen(),
    );
}

fn finish_setup(
    ui: &mut cursive::Cursive,
    config: &lqos_config::Config,
    mut event_log: Vec<String>,
) {
    if let Err(err) = setup_actions::persist_setup_success(config, &mut event_log) {
        ui.add_layer(
            Dialog::around(TextView::new(format!(
                "Configuration was written, but setup state could not be persisted:\n{err:#}\n\nSetup cannot be marked complete until bootstrap_state.json is saved successfully."
            )))
            .title("Unable To Persist Setup State")
            .button("OK", |s| {
                s.pop_layer();
            }),
        );
        return;
    }
    match service_handoff::schedule_runtime_handoff() {
        Ok(notice) => event_log.push(notice.message),
        Err(err) => event_log.push(format!("WARNING: {err:#}")),
    }

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

fn finalize(ui: &mut cursive::Cursive) {
    match hotfix::status() {
        Ok(status) if status.required => {
            ui.add_layer(
                Dialog::around(TextView::new(format!(
                    "{}\n\nInstall the Noble systemd hotfix before completing setup.",
                    status.detail
                )))
                .title("Hotfix Required")
                .button("Install Hotfix", |s| {
                    s.pop_layer();
                    install_hotfix_from_tui(s);
                })
                .button("Cancel", |s| {
                    s.pop_layer();
                }),
            );
            return;
        }
        Ok(_) => {}
        Err(err) => {
            ui.add_layer(
                Dialog::around(TextView::new(format!(
                    "Unable to determine hotfix status:\n{err:#}"
                )))
                .title("Hotfix Check Failed")
                .button("OK", |s| {
                    s.pop_layer();
                }),
            );
            return;
        }
    }

    // If we cannot load the config but a file exists, warn the user that
    // continuing will replace it after saving a backup.
    let config_path = Path::new("/etc/lqos.conf");
    let load_result = lqos_config::load_config();
    if load_result.is_err() && config_path.exists() {
        let msg = "An existing configuration file was found at /etc/lqos.conf,\n\
but it could not be read or parsed.\n\n\
Press Continue to replace it with a new configuration using defaults.\n\
LibreQoS will first try to save a backup to /etc/lqos.conf.setupbackup.\n\n\
Press Cancel to exit and investigate."
            .to_string();

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

fn show_hotfix_dialog(ui: &mut cursive::Cursive) {
    match hotfix::status() {
        Ok(status) => {
            let detail = if status.required {
                format!(
                    "{}\n\nThe hotfix is required before setup can complete.",
                    status.detail
                )
            } else {
                status.detail
            };
            let mut dialog = Dialog::around(TextView::new(detail)).title("Systemd Hotfix");
            if status.required {
                dialog.add_button("Install Hotfix", |s| {
                    s.pop_layer();
                    install_hotfix_from_tui(s);
                });
            }
            dialog.add_button("OK", |s| {
                s.pop_layer();
            });
            ui.add_layer(dialog);
        }
        Err(err) => ui.add_layer(
            Dialog::around(TextView::new(format!(
                "Unable to determine hotfix status:\n{err:#}"
            )))
            .title("Systemd Hotfix")
            .button("OK", |s| {
                s.pop_layer();
            }),
        ),
    }
}

fn install_hotfix_from_tui(ui: &mut cursive::Cursive) {
    match hotfix::install() {
        Ok(result) => {
            ui.add_layer(
                Dialog::around(TextView::new(format!(
                    "{}\n\nOpen the details only if you need the installer log.\n\n{}",
                    result.summary, result.detail
                )))
                .title("Hotfix Installed")
                .button("OK", |s| {
                    s.pop_layer();
                }),
            );
        }
        Err(err) => {
            ui.add_layer(
                Dialog::around(TextView::new(format!("Unable to install hotfix:\n{err:#}")))
                    .title("Hotfix Install Failed")
                    .button("OK", |s| {
                        s.pop_layer();
                    }),
            );
        }
    }
}

fn continue_finalize(ui: &mut cursive::Cursive) {
    match setup_actions::prepare_commit() {
        Ok(setup_actions::CommitOutcome::Complete(success)) => {
            finish_setup(ui, &success.config, success.event_log);
        }
        Ok(setup_actions::CommitOutcome::Pending(pending)) => {
            let operation_id = pending.operation_id.clone();
            let revert_operation_id = pending.operation_id.clone();
            ui.add_layer(
                Dialog::around(TextView::new(pending.prompt))
                    .title("Confirm Netplan Change")
                    .button(
                        "Confirm",
                        move |s| match setup_actions::confirm_pending_commit(&operation_id) {
                            Ok(success) => {
                                s.pop_layer();
                                finish_setup(s, &success.config, success.event_log);
                            }
                            Err(err) => {
                                s.add_layer(
                                    Dialog::around(TextView::new(format!(
                                        "Unable to confirm the pending network change:\n{err:#}"
                                    )))
                                    .title("Helper Error")
                                    .button("OK", |s| {
                                        s.pop_layer();
                                    }),
                                );
                            }
                        },
                    )
                    .button(
                        "Revert",
                        move |s| match setup_actions::revert_pending_commit(&revert_operation_id) {
                            Ok(message) => {
                                s.add_layer(
                                    Dialog::around(TextView::new(message))
                                        .title("Network Change Reverted")
                                        .button("OK", |s| {
                                            s.pop_layer();
                                            s.pop_layer();
                                        }),
                                );
                            }
                            Err(err) => {
                                s.add_layer(
                                    Dialog::around(TextView::new(format!(
                                        "Unable to revert the pending network change:\n{err:#}"
                                    )))
                                    .title("Helper Error")
                                    .button("OK", |s| {
                                        s.pop_layer();
                                    }),
                                );
                            }
                        },
                    )
                    .full_screen(),
            );
        }
        Err(err) => {
            ui.add_layer(
                Dialog::around(TextView::new(format!("{err:#}")))
                    .title("Unable To Save Setup")
                    .button("OK", |s| {
                        s.pop_layer();
                    }),
            );
        }
    }
}
