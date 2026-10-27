mod bandwidth;
mod bridge_mode;
mod config_builder;
mod interfaces;
mod ip_range;
mod preflight;
mod service_handoff;
mod setup_actions;
#[cfg(test)]
mod test_support;
mod webusers;

use clap::{Parser, Subcommand};
use lqos_setup::{bootstrap, config_migrate, hotfix};
use std::path::Path;

use bandwidth::bandwidth_view;
use config_builder::{CURRENT_CONFIG, existing_config_load_error};
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
    /// Print the current setup status
    Status,
    /// Print the current Ubuntu 24.04 systemd hotfix status
    HotfixStatus,
    /// Install the Ubuntu 24.04 systemd hotfix without prompting for reboot
    InstallHotfix,
    /// Exit 0 when runtime services should start, or 1 when setup is still required
    IsReady,
    /// Internal helper: print the package postinst action token
    PostinstAction,
    /// Internal helper: stop setup and activate runtime services now
    ActivateRuntime,
    /// Internal helper: stop runtime and activate the first-run setup service now
    ActivateSetup,
    /// Internal helper: the first-run setup-state MARKER (the boot-ladder gate)
    FirstRunMarker,
    /// Apply the host CPU power posture (`[power] profile`; turbo is always
    /// off). Bare metal applies live; a VM prints the host-side steps and
    /// writes nothing (`docs/power_profile_work_order.md`).
    PowerApply {
        /// Print the plan without writing anything.
        #[arg(long, default_value_t = false)]
        dry_run: bool,
    },
    /// Bind (or roll back) the dataplane NICs to the DPDK driver per the plan
    /// (`docs/nic_determinism_card.md` §2b).
    BindNics {
        /// vfio | vfio-noiommu | uio (default: auto from the host facts).
        #[arg(long)]
        mode: Option<String>,
        /// Roll back: rebind the NICs to the original kernel driver.
        #[arg(long, default_value_t = false)]
        unbind: bool,
        /// The original kernel driver to restore on --unbind (default: the one
        /// recorded in bridge.env).
        #[arg(long)]
        original_driver: Option<String>,
    },
    /// Derive + write the managed bridge.env.d ports plan (the zero-manual-ports
    /// B.4 wiring): reads the link demand from lqos.conf, the HT-collapsed
    /// physical core count from sysfs, the NIC queue fact, and the operator's
    /// `[queues] workers` override, then atomically writes the managed drop-in.
    PortsPlan {
        /// WAN PCI BDF (as recorded in bridge.env).
        #[arg(long)]
        wan: String,
        /// LAN PCI BDF.
        #[arg(long)]
        lan: String,
        /// NIC/VM RX queue count (the A0 multiqueue fact). Omit to detect it
        /// from the WAN netdev's sysfs (never a manual act).
        #[arg(long)]
        queues: Option<u32>,
        /// The WAN netdev's kernel driver (default: detected from --wan-dev, or
        /// the persisted LQOS_DPDK_ORIG_WAN_DRIVER).
        #[arg(long)]
        driver: Option<String>,
        /// Print the plan; do not write.
        #[arg(long, default_value_t = false)]
        dry_run: bool,
    },
}

/// Derive + write the managed ports plan (B.4 wiring). Returns a process code.
fn run_ports_plan(
    wan: &str,
    lan: &str,
    queues: Option<u32>,
    driver: Option<String>,
    dry_run: bool,
) -> i32 {
    use lqos_setup::port_class::{
        gather_facts, managed_env_block_with_workers, plan, write_managed_ports_plan_at,
        MANAGED_DROPIN,
    };
 // (the implementer (1c)) ONE gather shared with the setup flow: the link demand
    // (down+up, the shared pool), the HT-collapsed cores, the A0 queue fact, the
    // live override — never a second drifted derive.
    let cfg = match lqos_config::load_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ports-plan: cannot load lqos.conf: {e}");
            return 1;
        }
    };
    let mut facts = match gather_facts(&cfg, wan) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("ports-plan: {e}");
            return 1;
        }
    };
    if !facts.topology_ok {
        eprintln!(
            "ports-plan: WARNING cpu topology unavailable under /sys — using the logical count \
             {}; the derived worker count may be over/under-stated on an HT or multi-socket host",
            facts.physical_cores
        );
    }
    if let Some(q) = queues {
        facts.nic_queue_count = q;
    }
    let driver = driver
        .or_else(|| lqos_setup::bootstrap::bridge_env_value("LQOS_DPDK_ORIG_WAN_DRIVER"))
        .unwrap_or_else(|| "mlx5_core".to_string());
 // (the implementer (3)) NEVER hardcode iommu_present: an Intel no-IOMMU host must
    // get VfioNoiommu, else the bind fails.
    let Some(plan) = plan(&driver, lqos_setup::port_class::iommu_present()) else {
        eprintln!("ports-plan: unsupported NIC driver '{driver}' (Tier 3: no managed plan)");
        return 1;
    };
 // (the implementer (4)) the dry-run takes the SAME override-aware path as the write,
    // or it would print a different count than the file.
    let workers = facts.workers_override.unwrap_or_else(|| {
        lqos_setup::port_class::derived_workers(
            facts.link_mbps,
            facts.physical_cores,
            facts.nic_queue_count,
        )
    });
    // `lqos-74t.42`: the install-time VM/virtio warning on stderr (the persistent
    // copy lives in the managed block's `# WARNING` lines).
    if let Some(warn) = lqos_setup::port_class::virtio_worker_warning(plan.class, workers) {
        eprintln!("ports-plan: {warn}");
    }
    if dry_run {
        let body = managed_env_block_with_workers(&plan, wan, lan, &facts, workers);
        print!("{body}");
        return 0;
    }
    match write_managed_ports_plan_at(std::path::Path::new(MANAGED_DROPIN), &plan, wan, lan, &facts)
    {
        Ok(out) => {
            println!(
                "ports-plan: wrote {MANAGED_DROPIN} — workers={}{} (link={}Mbps cores={} queues={})",
                out.workers,
                if out.overridden { " (override)" } else { "" },
                facts.link_mbps,
                facts.physical_cores,
                facts.nic_queue_count
            );
            0
        }
        Err(e) => {
            eprintln!("ports-plan: write failed: {e}");
            1
        }
    }
}

fn config_exists() -> bool {
    let config_path = Path::new("/etc/lqos.conf");
    config_path.exists()
}

fn can_load_config() -> bool {
    lqos_config::load_config().is_ok()
}

/// The first-run SETUP-STATE MARKER (the boot-ladder gate; the setup-CLI
/// switch). Exits 0 when setup is complete, 1 (fail-visible) when the operator
/// must run the interactive CLI over SSH. The unit is a marker, not a web
/// servant — no 9123 wizard binds.
fn run_first_run_marker() -> i32 {
    match bootstrap::runtime_services_should_start() {
        Ok(true) => {
            println!("LibreQoS setup is complete; the runtime ladder may climb.");
            0
        }
        Ok(false) => {
            println!("LibreQoS first-run setup is INCOMPLETE.");
            println!("SSH in and run:  sudo lqos_setup tui");
            println!("(NIC pick/verify -> units -> activate-runtime)");
            1
        }
        Err(err) => {
            eprintln!("Unable to determine setup readiness: {err:#}");
            eprintln!("Run the interactive setup:  sudo lqos_setup tui");
            1
        }
    }
}

/// Bind (or roll back) the dataplane NICs per the card's §1 tree. The BDF pair
/// comes from the setup-written `bridge.env`; the mode defaults to the host
/// facts. Idempotent; every failure is a NAMED line.
/// The one pair-validity rule shared by the bind and unbind arms.
fn pair_valid_at(
    sysfs: &Path,
    pair: &lqos_setup::bootstrap::BridgeBdfPair,
    single: bool,
) -> Result<(), String> {
    if single {
        if !pair.wan.eq_ignore_ascii_case(&pair.lan) {
            return Err(format!(
                "[single_interface] bridge.env must carry the SAME BDF in both keys \
                 (found {} / {}); unbind and re-run setup to repair the pair",
                pair.wan, pair.lan
            ));
        }
        lqos_setup::port_class::validate_single_device_at(sysfs, &pair.wan)
    } else {
        lqos_setup::port_class::validate_bind_pair_at(sysfs, &pair.wan, &pair.lan)
    }
}

/// The rollback pair: prefer a VALID STATE pair (the bind fact), fall back to
/// the effective `bridge.env`/drop-in pair, and fail named if neither is
/// usable. A stale STATE (`virtio2/virtio3`) never blocks a recoverable
/// rollback; nothing is ever guessed.
fn choose_unbind_pair_at(
    sysfs: &Path,
    env_pair: Option<lqos_setup::bootstrap::BridgeBdfPair>,
    state_pair: Option<lqos_setup::bootstrap::BridgeBdfPair>,
    single_hint: Option<bool>,
) -> Result<lqos_setup::bootstrap::BridgeBdfPair, String> {
    if let Some(state) = state_pair {
        let single = single_hint.unwrap_or_else(|| state.wan.eq_ignore_ascii_case(&state.lan));
        if pair_valid_at(sysfs, &state, single).is_ok() {
            return Ok(state);
        }
    }
    if let Some(env) = env_pair {
        let single = single_hint.unwrap_or_else(|| env.wan.eq_ignore_ascii_case(&env.lan));
        pair_valid_at(sysfs, &env, single)?;
        return Ok(env);
    }
    Err("no usable BDF pair in bridge.env (incl. bridge.env.d) or the bind-originals \
         STATE — cannot restore the NICs without guessing; run setup before the bind"
        .to_string())
}

fn run_bind_nics(mode: Option<String>, unbind: bool, original_driver: Option<String>) -> i32 {
    use lqos_setup::bootstrap::{
        BridgeBdfPair, ENV_ORIGINAL_DRIVER, ENV_ORIG_LAN_DRIVER, ENV_ORIG_WAN_DRIVER,
        bridge_env_bdf_pair, bridge_env_value, set_bridge_env_value, state_original_driver,
        state_value,
    };
    use lqos_setup::nic_bind::{
        BindMode, PciSysfs, bind, capture_original, decide_mode, resolve_original, unbind_to,
    };

    // lqos-74t.37.8 B2: fail LOUD on a bus name (`virtioN`), a duplicate
    // function, or the transitional virtio class BEFORE touching sysfs. The
    // old basename resolver persisted `virtio2/virtio3`, which the bind could
    // never resolve.
    //
    // lqos-74t.37.7: an on-a-stick box legitimately persists the SAME BDF in
    // both keys. The single arm allows that (and validates the one device);
    // the two-port arm keeps rejecting it.
    let env_pair = bridge_env_bdf_pair();
    let state_pair = match (
        state_value("LQOS_DPDK_ORIG_WAN_BDF"),
        state_value("LQOS_DPDK_ORIG_LAN_BDF"),
    ) {
        (Some(wan), Some(lan)) => Some(BridgeBdfPair { wan, lan }),
        _ => None,
    };
    let single_hint = lqos_config::load_config()
        .ok()
        .map(|config| config.single_interface.is_some());
    let pair = if unbind {
        match choose_unbind_pair_at(Path::new("/sys"), env_pair, state_pair, single_hint) {
            Ok(pair) => pair,
            Err(error) => {
                eprintln!("bind-nics: {error}");
                return 1;
            }
        }
    } else {
        let Some(pair) = env_pair else {
            eprintln!(
                "No dataplane BDF pair in /etc/libreqos/bridge.env (incl. bridge.env.d) — \
                 run setup (the pair is written at the bind moment)."
            );
            return 1;
        };
        pair
    };
    // A broken/unmigrated config must not block a rollback: an EQUAL pair is
    // the on-a-stick invariant by construction, so fall back to that fact.
    let single = single_hint.unwrap_or_else(|| pair.wan.eq_ignore_ascii_case(&pair.lan));
    if let Err(error) = pair_valid_at(Path::new("/sys"), &pair, single) {
        eprintln!("bind-nics: {error}");
        return 1;
    }
    let sysfs = PciSysfs::system();

    if unbind {
        // The Rust --unbind precedence (the ONE rule; the packaged --restore has
        // no arg tier, so it is this chain minus the first tier):
        //   explicit --original-driver -> setup env per-port -> STATE -> singular
        // The explicit arg is FIRST: an operator override must beat a persisted
        // value (it used to be folded into `singular` and therefore LOST).
        let per_port = |key: &str, which: &str| {
            resolve_original(
                original_driver.as_deref(),
                bridge_env_value(key).as_deref(),
                state_original_driver(which).as_deref(),
                bridge_env_value(ENV_ORIGINAL_DRIVER).as_deref(),
            )
        };
        let wan_drv = per_port(ENV_ORIG_WAN_DRIVER, "WAN");
        let lan_drv = per_port(ENV_ORIG_LAN_DRIVER, "LAN");
        for (bdf, drv) in [(&pair.wan, wan_drv), (&pair.lan, lan_drv)] {
            let driver = drv.unwrap_or_else(|| "i40e".to_string());
            match unbind_to(&sysfs, bdf, &driver) {
                Ok(()) => println!("rollback: {bdf} rebound to {driver}"),
                Err(err) => {
                    eprintln!("rollback failed: {err}");
                    return 1;
                }
            }
        }
        return 0;
    }

    let mode = match mode.as_deref() {
        Some("vfio") => BindMode::Vfio,
        Some("vfio-noiommu") => BindMode::VfioNoiommu,
        Some("uio") => BindMode::Uio,
        Some(other) => {
            eprintln!("Unknown --mode {other} (expected vfio | vfio-noiommu | uio).");
            return 2;
        }
        None => {
            // Derive from the NIC CLASS, not the host's available drivers: a
            // virtio port needs `uio_pci_generic` (+ the 1af4 1041 new_id),
            // not vfio; an mlx5 stays in place. `decide_mode` alone would pick
            // vfio for a virtio VM (the .37.8 bind hole).
            let driver = lqos_setup::port_class::driver_of_bdf(&pair.wan)
                .filter(|driver| !lqos_setup::nic_bind::is_dpdk_driver(driver))
                .or_else(|| bridge_env_value(ENV_ORIG_WAN_DRIVER))
                .unwrap_or_else(|| "mlx5_core".to_string());
            match lqos_setup::port_class::plan(&driver, lqos_setup::port_class::iommu_present()) {
                Some(plan) => match plan.bind_mode {
                    Some(mode) => mode,
                    None => {
                        println!("bind-nics: {driver} stays in place (no bind required)");
                        return 0;
                    }
                },
                None => match decide_mode(&sysfs) {
                    Some(mode) => mode,
                    None => {
                        eprintln!(
                            "UNTESTED: no DPDK class for '{driver}' and neither vfio-pci nor \
                             uio_pci_generic is available on this host."
                        );
                        return 1;
                    }
                },
            }
        }
    };

    // Record the kernel driver(s) to restore on rollback (only once, and the
    // TRUE pre-bind driver per port — the netdev->driver fact does not survive
    // the bind). The singular key is kept for a same-driver host / older tools.
    // capture_original deliberately yields None when the port is already on a
    // DPDK driver (never record uio/vfio as the "original").
    let lan_now = capture_original(&sysfs, &pair.lan);
    let wan_now = capture_original(&sysfs, &pair.wan);
    // The persist is the removal safety net: a failed write is a NAMED error
    // (never `let _`), because --unbind/--restore would then have no original.
    let mut persist = |key: &str, driver: Option<&str>| {
        if bridge_env_value(key).is_some() {
            return;
        }
        if let Some(driver) = driver
            && let Err(err) = set_bridge_env_value(key, driver)
        {
            eprintln!(
                "WARN: could not persist {key}={driver} to bridge.env ({err}); the removal path \
                 will fall back to the singular/--original-driver value"
            );
        }
    };
    persist(ENV_ORIG_LAN_DRIVER, lan_now.as_deref());
    persist(ENV_ORIG_WAN_DRIVER, wan_now.as_deref());
    persist(
        ENV_ORIGINAL_DRIVER,
        lan_now.as_deref().or(wan_now.as_deref()),
    );

    for bdf in [&pair.lan, &pair.wan] {
        match bind(&sysfs, bdf, mode) {
            Ok(()) => println!("bound {bdf} to {}", mode.driver()),
            Err(err) => {
                eprintln!("bind failed: {err}");
                return 1;
            }
        }
    }
    0
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
        Some(Command::PostinstAction) => match bootstrap::classify_postinst_action() {
            Ok(action) => println!("{}", action.as_str()),
            Err(err) => {
                eprintln!("Unable to determine package postinst action: {err:#}");
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
        Some(Command::FirstRunMarker) => std::process::exit(run_first_run_marker()),
        Some(Command::PowerApply { dry_run }) => match lqos_setup::power::apply_live(dry_run, false) {
            Ok(report) => print!("{}", lqos_setup::power::render(&report)),
            Err(err) => {
                eprintln!("Unable to apply the power profile: {err:#}");
                std::process::exit(1);
            }
        },
        Some(Command::BindNics {
            mode,
            unbind,
            original_driver,
        }) => std::process::exit(run_bind_nics(mode, unbind, original_driver)),
        Some(Command::PortsPlan {
            wan,
            lan,
            queues,
            driver,
            dry_run,
        }) => std::process::exit(run_ports_plan(&wan, &lan, queues, driver, dry_run)),
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
    if let Some(message) = existing_config_load_error() {
        ui.add_layer(
            Dialog::around(TextView::new(message))
                .title("Existing Config Unreadable")
                .button("OK", |s| {
                    s.pop_layer();
                }),
        );
        return;
    }

    // Otherwise, proceed to saving normally.
    continue_finalize(ui);
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

#[cfg(test)]
mod cli_tests {
    use super::*;

    /// The setup-CLI switch (docs/setup_cli_card.md slice b): the 9123 setup
    /// wizard is gone. The `web` subcommand must NOT parse; `tui` must.
    #[test]
    fn a_stale_state_pair_falls_back_to_the_effective_env_pair() {
        use lqos_setup::bootstrap::BridgeBdfPair;
        let root = std::env::temp_dir().join(format!(
            "lqos-bind-unbind-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        for bdf in ["0000:00:13.0", "0000:00:14.0"] {
            let dir = root.join("bus/pci/devices").join(bdf);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("class"), "0x020000\n").unwrap();
        }
        let valid = BridgeBdfPair {
            wan: "0000:00:13.0".to_string(),
            lan: "0000:00:14.0".to_string(),
        };
        let stale = BridgeBdfPair {
            wan: "virtio2".to_string(),
            lan: "virtio3".to_string(),
        };
        // The .9 shape: STATE is stale, the effective env pair is valid.
        let chosen =
            choose_unbind_pair_at(&root, Some(valid.clone()), Some(stale.clone()), Some(false))
                .expect("falls back to the env pair");
        assert_eq!(chosen, valid);
        // A valid STATE wins (the bind fact).
        let chosen = choose_unbind_pair_at(
            &root,
            Some(valid.clone()),
            Some(valid.clone()),
            Some(false),
        )
        .unwrap();
        assert_eq!(chosen, valid);
        // Neither usable -> named failure, never a guess.
        let error = choose_unbind_pair_at(&root, Some(stale.clone()), Some(stale), Some(false))
            .expect_err("no usable pair");
        assert!(error.contains("not a PCI BDF"), "{error}");
        let error = choose_unbind_pair_at(&root, None, None, None)
            .expect_err("nothing to restore from");
        assert!(error.contains("without guessing"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_setup_web_subcommand_is_gone() {
        assert!(
            Args::try_parse_from(["lqos_setup", "web"]).is_err(),
            "the setup web subcommand must be removed (the 9123 wizard)"
        );
        assert!(Args::try_parse_from(["lqos_setup", "tui"]).is_ok());
        assert!(Args::try_parse_from(["lqos_setup", "first-run-marker"]).is_ok());
        assert!(Args::try_parse_from(["lqos_setup", "bind-nics"]).is_ok());
        assert!(
            Args::try_parse_from(["lqos_setup", "bind-nics", "--mode", "vfio-noiommu"]).is_ok()
        );
        assert!(Args::try_parse_from(["lqos_setup", "bind-nics", "--unbind"]).is_ok());
    }
}
