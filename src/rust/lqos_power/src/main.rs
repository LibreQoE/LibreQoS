//! `libreqos-power-profile` — the host CPU power-profile CLI
//! (`docs/power_profile_work_order.md` §5).
//!
//! `libreqos-power-profile set <eco|balanced|performance>` writes the single
//! source (`lqos.conf` `[power] profile`) and applies it live.
//! `restore` puts back every recorded pre-existing value.
//! `status` reports the configured profile and the detected platform.

use clap::{Parser, Subcommand};

use lqos_power::apply::{ApplyRequest, ORIGINALS_PATH};
use lqos_power::config_edit;
use lqos_power::platform::Platform;
use lqos_power::profile::PowerProfile;
use lqos_power::sysfs::{RealCommands, RealSysfs};
use lqos_power::{ApplyReport, TurboOff, apply, restore};

#[derive(Parser)]
#[command(
    name = "libreqos-power-profile",
    about = "LibreQoS host CPU power posture (turbo is always off)"
)]
struct Cli {
    /// lqos.conf path (default: the resolved LQOS_CONFIG / canonical path).
    #[arg(long)]
    config: Option<String>,
    /// The reversibility record path.
    #[arg(long, default_value = ORIGINALS_PATH)]
    originals: String,
    /// Plan and report without writing anything.
    #[arg(long, default_value_t = false)]
    dry_run: bool,
    /// Proceed despite an active tuned/cpupower/power-profiles-daemon (counted
    /// + warned; the conflict otherwise REFUSES).
    #[arg(long, default_value_t = false)]
    force: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Select a profile: write `[power] profile` and apply it live.
    Set { profile: String },
    /// Apply the profile already named in lqos.conf.
    Apply,
    /// Restore every recorded pre-existing value and clear the record.
    Restore,
    /// Report the configured profile and the detected platform.
    Status,
}

fn report(report: &ApplyReport) {
    if !report.conflicts.is_empty() {
        eprintln!(
            "WARNING: {} is active and may revert/fight the profile — name it to the operator",
            report.conflicts.join(", ")
        );
    }
    if report.virtualized {
        println!(
            "This host is a VM: the guest cannot set the host CPU policy. \
             Run these on the bare-metal HOST:"
        );
        for step in &report.host_steps {
            println!("{step}");
        }
        return;
    }
    let verb = if report.dry_run {
        "would apply"
    } else {
        "applied"
    };
    println!(
        "{verb} power profile '{}' ({} setting(s), {} changed):",
        report.profile,
        report.writes.len(),
        report.changed()
    );
    println!("turbo: {}", report.turbo_off.describe());
    if matches!(report.turbo_off, TurboOff::Unsupported) {
        eprintln!(
            "WARNING: this host exposes no turbo knob and no nominal frequency; \
             turbo-off CANNOT be enforced by the power profile"
        );
    }
    for w in &report.writes {
        println!(
            "  {:?}: {} {} -> {}",
            w.facet,
            w.path,
            w.old.as_deref().unwrap_or("<absent>"),
            w.new
        );
    }
    if report.forced {
        println!("(forced despite an active conflicting control service)");
    }
    if !report.unverified.is_empty() {
        eprintln!(
            "WARNING: {} setting(s) did NOT stick — a conflicting daemon rewrote them:",
            report.unverified.len()
        );
        for (path, expected, actual) in &report.unverified {
            eprintln!("  {path} expected {expected} actual {actual}");
        }
    }
    if report.dry_run {
        println!("(dry-run: nothing was written)");
    }
}

fn main() {
    let cli = Cli::parse();
    let config = cli.config.unwrap_or_else(config_edit::config_path);

    let result = match cli.command {
        Command::Set { profile } => {
            let profile: PowerProfile = match profile.parse() {
                Ok(p) => p,
                Err(err) => {
                    eprintln!("error: {err}");
                    std::process::exit(2);
                }
            };
            if cli.dry_run {
                println!("dry-run: would write [power] profile = \"{profile}\" to {config}");
            } else {
                match config_edit::set_profile(&config, profile) {
                    Ok(()) => println!("wrote [power] profile = \"{profile}\" to {config}"),
                    Err(err) => {
                        eprintln!("error: cannot write {config}: {err}");
                        std::process::exit(1);
                    }
                }
            }
            let cfg = config_edit::read_power_config(&config).unwrap_or_default();
            let request = ApplyRequest {
                profile,
                eco_max_freq_pct: cfg.eco_max_freq_pct,
                force: cli.force,
                dry_run: cli.dry_run,
            };
            apply(&request, &RealSysfs, &RealCommands, &cli.originals)
        }
        Command::Apply => {
            let cfg = config_edit::read_power_config(&config).unwrap_or_default();
            let request = ApplyRequest {
                profile: cfg.profile,
                eco_max_freq_pct: cfg.eco_max_freq_pct,
                force: cli.force,
                dry_run: cli.dry_run,
            };
            apply(&request, &RealSysfs, &RealCommands, &cli.originals)
        }
        Command::Restore => match restore(&RealSysfs, &cli.originals) {
            Ok(n) => {
                println!("restored {n} value(s) from {}", cli.originals);
                return;
            }
            Err(err) => Err(err),
        },
        Command::Status => {
            let configured = config_edit::read_profile(&config);
            let platform = Platform::detect(&RealSysfs, &RealCommands);
            println!(
                "configured profile: {}",
                configured
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "<unset> (default balanced)".to_string())
            );
            println!(
                "platform: {} (driver {})",
                if platform.virtualized {
                    "VM"
                } else {
                    "bare metal"
                },
                platform.scaling_driver.as_deref().unwrap_or("<none>")
            );
            println!("turbo: {}", platform.turbo_off().describe());
            println!(
                "facets: turbo_knobs={} policies={} epp={} uncore={} cstates={}",
                platform.turbo_paths.len(),
                platform.policies.len(),
                platform.epp_paths.len(),
                platform.uncore_dirs.len(),
                platform.cstate_states.len(),
            );
            return;
        }
    };

    match result {
        Ok(outcome) => report(&outcome),
        Err(err) => {
            eprintln!("error: {err:#}");
            std::process::exit(1);
        }
    }
}
