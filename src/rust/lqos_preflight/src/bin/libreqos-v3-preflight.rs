//! The standalone v2→v3 preflight checker (`libreqos-v3-preflight`).
//!
//! Runs ON the v2 box — before any v3 install — and prints a
//! PASS/WARN(list)/BLOCK(list) verdict (the card's §1). Built static so it
//! runs on the v2 fleet's older glibc vintages.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use lqos_preflight::{
    Severity, UnitPresence, Verdict, check_fetch, check_repo, compose_with_mode, hardware,
};

/// Qualify a v2 box for the v3 upgrade before installing.
#[derive(Parser, Debug)]
#[command(name = "libreqos-v3-preflight", version, about)]
struct Args {
    /// The directory holding the compat pair / integration artifacts
    /// (`network.json` + `ShapedDevices.csv`, or `state/topology/`).
    #[arg(long)]
    pair_dir: Option<PathBuf>,

    /// Pre-install/preinst mode: expected fresh facts are NOTE lines and do
    /// not flip PASS to WARN. Also `LQOS_PREFLIGHT_PRE_INSTALL=1`.
    #[arg(long)]
    pre_install: bool,
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
    if let Some(fetch) = &verdict.fetch
        && let Some((host, port)) = &fetch.endpoint
    {
        println!("  dataplane fetch: {host}:{port} (reachability checked at install)");
    }
    if let Some(repo) = &verdict.repo
        && let (Some(url), Some(suite)) = (&repo.url, &repo.suite)
    {
        println!(
            "  apt archive: {url} {suite}{}",
            repo.candidate
                .as_deref()
                .map(|c| format!(" (candidate {c})"))
                .unwrap_or_default()
        );
    }
    if let Some(install) = &verdict.install
        && install.warning().is_none()
    {
        println!("  install integrity: supported (package-owned)");
    }
    if let Some(seam) = &verdict.seam
        && seam.installed.is_some()
        && seam.aligned
    {
        println!(
            "  control seam: installed lqosd protocol {} aligned",
            seam.expected
        );
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
    for note in &verdict.notes {
        println!("  NOTE: {note}");
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

/// On-a-stick: one interface carries WAN and LAN (`single_interface`, or a
/// bridge whose to_internet == to_network). One supported port is then a
/// WARN/pass (with caveats), never the inline-pair BLOCK.
/// `bridge.conf` sits beside the resolved config; its `interface_topology.mode`
/// is the setup-written on-a-stick marker.
fn bridge_conf_stick() -> bool {
    let path = Path::new(&lqos_config::config_path())
        .parent()
        .map(|dir| dir.join("bridge.conf"))
        .unwrap_or_else(|| PathBuf::from("/etc/libreqos/bridge.conf"));
    std::fs::read_to_string(path)
        .ok()
        .is_some_and(|body| lqos_preflight::on_a_stick_bridge_conf(&body))
}

fn on_a_stick_from_config(config: &lqos_config::Config) -> bool {
    if config.single_interface.is_some() {
        return true;
    }
    config.bridge.as_ref().is_some_and(|bridge| {
        let wan = bridge.to_internet.trim();
        let lan = bridge.to_network.trim();
        !wan.is_empty() && wan == lan
    })
}

fn main() -> ExitCode {
    let args = Args::parse();
    let config = load_host_config();
    // A fresh host has no config yet: the dataplane-pair hardware block is a
    // warning until setup chooses the ports. An upgrade keeps the strict gate.
    let fresh_install = config.is_none()
        && !Path::new("/etc/lqos.conf").exists()
        && !Path::new("/etc/libreqos/lqos.conf").exists();
    let pre_install = args.pre_install
        || std::env::var("LQOS_PREFLIGHT_PRE_INSTALL")
            .ok()
            .is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"));
    let on_a_stick =
        config.as_ref().is_some_and(on_a_stick_from_config) || bridge_conf_stick();
    let mut verdict = compose_with_mode(
        &resolve_pair_dir(args.pair_dir),
        hardware::probe_with_policy(fresh_install, on_a_stick),
        units_probe,
        fresh_install,
        pre_install,
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
    if let Some(config) = config {
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
