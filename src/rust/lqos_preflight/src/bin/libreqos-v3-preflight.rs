//! The standalone v2→v3 preflight checker (`libreqos-v3-preflight`).
//!
//! Runs ON the v2 box — before any v3 install — and prints a
//! PASS/WARN(list)/BLOCK(list) verdict (the card's §1). Built static so it
//! runs on the v2 fleet's older glibc vintages.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use lqos_preflight::{
    Severity, UnitPresence, Verdict, check_fetch, check_repo, compose_with_fresh, hardware,
};

/// Qualify a v2 box for the v3 upgrade before installing.
#[derive(Parser, Debug)]
#[command(name = "libreqos-v3-preflight", version, about)]
struct Args {
    /// The directory holding the compat pair / integration artifacts
    /// (`network.json` + `ShapedDevices.csv`, or `state/topology/`).
    #[arg(long)]
    pair_dir: Option<PathBuf>,
}

/// Resolve the directory to check: an explicit `--pair-dir` wins; otherwise
/// probe the v3 state root (`/opt/libreqos`) first, then `/etc/libreqos`.
/// The v3 state root is where the integration import artifacts live
/// (`state/topology/topology_import.json`), so a bare run finds them.
fn resolve_pair_dir(arg: Option<PathBuf>) -> PathBuf {
    if let Some(dir) = arg {
        return dir;
    }
    for candidate in ["/opt/libreqos", "/etc/libreqos"] {
        if Path::new(candidate).exists() {
            return PathBuf::from(candidate);
        }
    }
    PathBuf::from("/etc/libreqos")
}

/// Report unit presence from the systemd unit directories. Presence only —
/// active-state needs `systemctl`, kept out of the pure core.
fn units_probe(unit: &str) -> UnitPresence {
    for dir in [
        "/etc/systemd/system",
        "/lib/systemd/system",
        "/usr/lib/systemd/system",
    ] {
        if std::path::Path::new(dir).join(unit).exists() {
            return UnitPresence::Inactive;
        }
    }
    UnitPresence::Absent
}

fn label(severity: Severity) -> &'static str {
    match severity {
        Severity::Pass => "PASS",
        Severity::Warn => "WARN",
        Severity::Block => "BLOCK",
    }
}

fn print_verdict(verdict: &Verdict) {
    println!("libreqos-v3-preflight: {}", label(verdict.severity));
    println!(
        "  hardware: {:?} ({} dataplane port(s), kernel {:?})",
        verdict.hardware.class, verdict.hardware.dataplane_ports, verdict.hardware.kernel
    );
    match verdict.circuit_count {
        Some(count) => println!(
            "  circuits: at least {count} (unlicensed cap {})",
            verdict.circuit_limit
        ),
        None => println!("  circuits: unavailable"),
    }
    if let Some(source) = &verdict.compile.source {
        println!("  topology: validated source={source}");
    }
    let present: Vec<&str> = verdict
        .units
        .iter()
        .filter(|row| row.is_present())
        .map(|row| row.unit)
        .collect();
    println!("  pre-existing units: {}", present.join(", "));
    if let Some(os) = &verdict.os {
        println!(
            "  os: {} (VERSION_ID {})",
            os.label(),
            os.version_id.as_deref().unwrap_or("unknown")
        );
        if let Some(note) = lqos_preflight::os::hotfix_advisory(os) {
            println!("  note: {note}");
        }
    }
    if let Some(fetch) = &verdict.fetch {
        match &fetch.endpoint {
            Some((host, port)) => {
                println!("  dataplane fetch: {host}:{port} (reachability checked at install)")
            }
            None => println!("  dataplane fetch: MISCONFIGURED ({})", fetch.url),
        }
    }
    if let Some(repo) = &verdict.repo {
        match (&repo.url, &repo.suite) {
            (Some(url), Some(suite)) => println!(
                "  apt archive: {url} {suite}{}",
                repo.candidate
                    .as_deref()
                    .map(|c| format!(" (candidate {c})"))
                    .unwrap_or_default()
            ),
            _ => println!("  apt archive: not configured ({})", repo.source_file),
        }
    }
    if let Some(install) = &verdict.install {
        match install.warning() {
            Some(warning) => println!("  install integrity: UNSUPPORTED — {warning}"),
            None => println!("  install integrity: supported (package-owned)"),
        }
    }
    if let Some(seam) = &verdict.seam {
        match seam.installed {
            Some(installed) => println!(
                "  control seam: installed lqosd protocol {installed} vs expected {} ({})",
                seam.expected,
                if seam.aligned { "aligned" } else { "SKEWED" }
            ),
            None => println!("  control seam: no readable installed lqosd"),
        }
    }
    if let Some(netplan) = &verdict.netplan {
        match netplan.conflicting.first() {
            Some(bridge) => println!(
                "  netplan: bridge {} over {} (adoption required)",
                bridge.name,
                netplan.pair.join("+")
            ),
            None => println!("  netplan: no conflicting bridges"),
        }
    }
    if let Some(plan) = &verdict.nic_plan {
        println!("  nic plan: pair {}", plan.pair.join("+"));
        println!("  nic plan: orientation {}", plan.orientation());
        println!("  nic plan: bind mode {}", plan.mode.label());
        println!(
            "  nic plan: PMD path {} ({})",
            plan.pmd_dir,
            if plan.pmd_present {
                "present"
            } else {
                "installed with the dataplane"
            }
        );
    }
    for warning in &verdict.warnings {
        println!("  WARN: {warning}");
    }
    for block in &verdict.blocks {
        println!("  BLOCK: {block}");
    }
}

/// Load the host config for the pre-install arms. This runs on a v2 box, whose
/// config is still `/etc/lqos.conf`; the v3 loader default moved to
/// `/etc/libreqos/lqos.conf`, so try the resolved path first, then the legacy
/// path (the migration writes the v3 path only at install).
fn load_host_config() -> Option<lqos_config::Config> {
    let primary = lqos_config::config_path();
    for candidate in [primary.as_str(), "/etc/lqos.conf"] {
        if !Path::new(candidate).exists() {
            continue;
        }
        // SAFETY: single-threaded at startup, before any config read.
        unsafe { std::env::set_var("LQOS_CONFIG", candidate) };
        lqos_config::clear_cached_config();
        if let Ok(config) = lqos_config::load_config() {
            return Some((*config).clone());
        }
    }
    None
}

fn main() -> ExitCode {
    let args = Args::parse();
    // A fresh host has no config yet: the dataplane-pair hardware block is a
    // warning until setup chooses the ports. An upgrade keeps the strict gate.
    let fresh_install = !Path::new("/etc/lqos.conf").exists()
        && !Path::new("/etc/libreqos/lqos.conf").exists();
    let mut verdict = compose_with_fresh(
        &resolve_pair_dir(args.pair_dir),
        hardware::probe(),
        units_probe,
        fresh_install,
    )
    .with_os(lqos_preflight::os::check())
    .with_fetch(check_fetch())
    .with_repo(check_repo());
    let installed_lqosd = std::path::Path::new(lqos_preflight::INSTALLED_LQOSD);
    if installed_lqosd.exists() {
        verdict = verdict.with_seam(lqos_preflight::seam::probe_installed(installed_lqosd));
    }
    // AVAIL 0(e) (install_integrity_preflight_card.md): detect a hand-rolled
    // (non-package-owned) v3 tree — REPORT/health degrade only, never a BLOCK.
    verdict = verdict.with_install(lqos_preflight::install::assess(
        &lqos_preflight::install::CommandDpkg,
        lqos_preflight::install::INTEGRITY_SET,
    ));
    // The netplan arm needs the dataplane pair from the host config; without a
    // readable config there is no pair to intersect, so the arm is skipped.
    if let Some(config) = load_host_config() {
        let pair = lqos_preflight::pair_from_config(&config);
        verdict = verdict
            .with_netplan(lqos_preflight::netplan::check(&pair))
            .with_nic_plan(lqos_preflight::bindplan::check(&pair));
    }
    print_verdict(&verdict);
    match verdict.severity {
        Severity::Pass => ExitCode::SUCCESS,
        Severity::Warn => ExitCode::from(1),
        Severity::Block => ExitCode::from(2),
    }
}
