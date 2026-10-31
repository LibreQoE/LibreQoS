//! The PASS/WARN/BLOCK composition (the card's §1 output).
//!
//! Folds the pure checks into one verdict: the dry-run compile is a BLOCK on
//! error, the circuit count over the unlicensed cap is a WARN, and the
//! pre-existing unit census is informational (the installer's swap/rollback
//! map). The hardware and fetch-prerequisite checks compose here in S2.

use std::path::Path;

use lqos_utils::mapped_circuits::DEFAULT_MAPPED_CIRCUIT_LIMIT;

use crate::bindplan::NicPlan;
use crate::hardware::HardwareVerdict;
use crate::install::InstallIntegrityReport;
use crate::netplan::NetplanReport;
use crate::os::{self, OsRelease};
use crate::prereq::{FetchPrereq, RepoConfig};
use crate::seam::SeamReport;
use crate::topology::{
    CompileReport, NO_TOPOLOGY_SOURCE, count_discovered_circuits, validate_pair,
};
use crate::units::{UnitPresence, UnitRow, inventory};

/// The overall verdict class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// No errors or warnings.
    Pass,
    /// Warnings only; the upgrade may proceed.
    Warn,
    /// At least one blocking error; the upgrade must not proceed.
    Block,
}

/// The composed preflight verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    /// The overall class (the worst of the checks).
    pub severity: Severity,
    /// The dry-run compile result.
    pub compile: CompileReport,
    /// The raw `ShapedDevices.csv` row count, when it could be read.
    pub circuit_count: Option<usize>,
    /// The unlicensed mapped-circuit cap used for the comparison.
    pub circuit_limit: u64,
    /// Non-fatal advisories (the licensing over-cap warning, etc.).
    pub warnings: Vec<String>,
    /// Fatal errors; a non-empty list is a BLOCK.
    pub blocks: Vec<String>,
    /// The hardware verdict (the §1.1 checks).
    pub hardware: HardwareVerdict,
    /// The OS-floor release (the §1.4 units/disk/OS arm), when attached.
    pub os: Option<OsRelease>,
    /// The netplan-bridge adoption scan (the upgrade card §2a), when attached.
    pub netplan: Option<NetplanReport>,
    /// The NIC determinism plan (the nic-determinism card §2a), when attached.
    pub nic_plan: Option<NicPlan>,
    /// The split-fetch prerequisite (the §1.4 URL check), when attached.
    pub fetch: Option<FetchPrereq>,
    /// The apt archive check (the repo-check arm), when attached.
    pub repo: Option<RepoConfig>,
    /// The control-seam alignment check, when attached.
    pub seam: Option<SeamReport>,
    /// The install-integrity check (card §B1-B3), when attached.
    pub install: Option<InstallIntegrityReport>,
    /// The pre-existing install state (informational; the rollback map).
    pub units: Vec<UnitRow>,
}

impl Verdict {
    /// True when the verdict blocks the upgrade.
    pub fn is_blocked(&self) -> bool {
        matches!(self.severity, Severity::Block)
    }

    /// Attach the OS-floor check (the §1.4 units/disk/OS arm). A host below
    /// the glibc-2.39 floor (Ubuntu 24.04+) is a BLOCK: the v3 binaries are
    /// built against glibc 2.39 and would otherwise install and then fail at
    /// runtime with an unhelpful loader error.
    pub fn with_os(mut self, release: OsRelease) -> Self {
        if let Some(block) = os::floor_block(&release) {
            self.blocks.push(block);
            self.severity = Severity::Block;
        }
        self.os = Some(release);
        self
    }

    /// Attach the netplan-bridge adoption scan (the upgrade card §2a). A
    /// bridge over a dataplane interface is a WARN naming the bridge + members
    /// (the upgrade will re-own those NICs for DPDK — adopt the bridge first);
    /// it is LOUDER when management appears to ride the bridge or a member (the
    /// operator-console-required class). Detect-only: never a BLOCK, never
    /// auto-adopted (card §4).
    pub fn with_netplan(mut self, report: NetplanReport) -> Self {
        let mut warned = false;
        for bridge in &report.conflicting {
            let detail = format!(
                "the netplan bridge {} ({}) enslaves dataplane interface(s) [{}]; the upgrade \
                 re-owns these NICs for DPDK — run the network-mode adoption first (the operator \
                 flow, transactional with a backup id) before binding",
                bridge.name,
                bridge.file.display(),
                bridge.members.join(", ")
            );
            if bridge.management_at_risk {
                self.warnings.push(format!(
                    "OPERATOR-CONSOLE REQUIRED — management appears to ride the bridge: {detail}"
                ));
            } else {
                self.warnings.push(detail);
            }
            warned = true;
        }
        if warned && matches!(self.severity, Severity::Pass) {
            self.severity = Severity::Warn;
        }
        self.netplan = Some(report);
        self
    }

    /// Attach the NIC determinism plan (the nic-determinism card §2a). An
    /// unresolvable pair or an UNTESTED bind-mode combination is a WARN: the
    /// operator must see the honest plan before approving the upgrade.
    pub fn with_nic_plan(mut self, plan: NicPlan) -> Self {
        let mut warned = false;
        if plan.pair.len() < 2 {
            self.warnings.push(
                "the NIC plan could not resolve a dataplane pair from the host config; check \
                 [bridge] to_internet/to_network (no dataplane ports can be bound)"
                    .to_string(),
            );
            warned = true;
        } else if plan.bdfs.len() != 2 {
            self.warnings.push(
                "the dataplane pair could not be resolved to PCI BDFs (the NICs are already \
                 bound, or are not PCI pair); the setup bind will re-derive them pre-bind if \
                 possible"
                    .to_string(),
            );
            warned = true;
        }
        if plan.mode.is_untested() {
            self.warnings.push(format!(
                "the bind-mode combination is UNTESTED on this host: {}",
                plan.mode.label()
            ));
            warned = true;
        }
        if warned && matches!(self.severity, Severity::Pass) {
            self.severity = Severity::Warn;
        }
        self.nic_plan = Some(plan);
        self
    }

    /// Attach the split-fetch prerequisite. A malformed/unset artifact URL is
    /// a WARN: the install stays non-bricking and the bridge simply stays
    /// inactive until a valid URL reaches the fetch hook.
    pub fn with_fetch(mut self, fetch: FetchPrereq) -> Self {
        if !fetch.is_configured() {
            self.warnings.push(format!(
                "the dataplane artifact URL is not well-formed ('{}'); the bridge will not fetch",
                fetch.url
            ));
            if matches!(self.severity, Severity::Pass) {
                self.severity = Severity::Warn;
            }
        }
        self.fetch = Some(fetch);
        self
    }

    /// Attach the apt archive check (the repo-check arm). A missing or
    /// unparsable archive entry is a WARN: the dataplane cannot be fetched and
    /// the bridge stays inactive until the archive is configured.
    pub fn with_repo(mut self, repo: RepoConfig) -> Self {
        let mut warned = false;
        if !repo.is_configured() {
            self.warnings.push(format!(
                "the LibreQoS archive is not configured ({}); the dataplane cannot be fetched \
                 and the bridge will stay inactive",
                repo.source_file
            ));
            warned = true;
        } else if repo
            .candidate
            .as_deref()
            .is_none_or(|candidate| candidate == "none")
        {
            self.warnings.push(format!(
                "the dataplane package is not available from the {:?} suite",
                repo.suite
            ));
            warned = true;
        }
        if warned && matches!(self.severity, Severity::Pass) {
            self.severity = Severity::Warn;
        }
        self.repo = Some(repo);
        self
    }

    /// Attach the control-seam alignment check. A protocol skew between the
    /// installed lqosd and this package is a WARN: the bridge would attach
    /// with a magic/version mismatch and present as a dead bridge.
    pub fn with_seam(mut self, seam: SeamReport) -> Self {
        if seam.installed.is_some() && !seam.aligned {
            self.warnings.push(format!(
                "the installed lqosd speaks wire protocol {:?} but this package expects {} — \
                 upgrade both together or the bridge will fail to attach (magic/version mismatch)",
                seam.installed, seam.expected
            ));
            if matches!(self.severity, Severity::Pass) {
                self.severity = Severity::Warn;
            }
        }
        self.seam = Some(seam);
        self
    }

    /// Attach the install-integrity check (card §B3). A hand-rolled or
    /// modified tree is a WARN "unsupported" — REPORT/health only; the box
    /// keeps shaping (never a BLOCK, §0/§7).
    pub fn with_install(mut self, report: InstallIntegrityReport) -> Self {
        if let Some(warning) = report.warning() {
            self.warnings.push(warning);
            if matches!(self.severity, Severity::Pass) {
                self.severity = Severity::Warn;
            }
        }
        self.install = Some(report);
        self
    }
}

/// Compose the checks over the pair in `pair_dir`.
///
/// `hardware` carries the §1.1 facts (so the pure core stays probe-free);
/// `units_probe` reports unit presence (so the census stays testable without
/// systemd). Neither the census nor the hardware probe runs systemd.
pub fn compose(
    pair_dir: &Path,
    hardware: HardwareVerdict,
    units_probe: impl Fn(&str) -> UnitPresence,
) -> Verdict {
    compose_with_fresh(pair_dir, hardware, units_probe, false)
}

/// [`compose`] with the fresh-install rule: a host with NO prior configuration
/// has not chosen its dataplane ports yet. The hardware "needs a PAIR" block
/// is therefore a WARNING, not a BLOCK (`lqos-74t.37.10`; a fresh 24.04 VM
/// installs, setup picks the ports). An upgrade with a config keeps the block.
pub fn compose_with_fresh(
    pair_dir: &Path,
    hardware: HardwareVerdict,
    units_probe: impl Fn(&str) -> UnitPresence,
    fresh_install: bool,
) -> Verdict {
    // `validate_pair` returns a WARNING (no source / half pair) or BLOCKs only
    // a PRESENT source that fails to compile (`lqos-74t.37.10`).
    let compile = validate_pair(pair_dir);
    let circuit_count = count_discovered_circuits(pair_dir);
    let circuit_limit = DEFAULT_MAPPED_CIRCUIT_LIMIT;

    let mut warnings = hardware.warnings.clone();
    let mut blocks = Vec::new();
    for block in hardware.blocks.clone() {
        if fresh_install && block.contains("inline deployment needs a PAIR") {
            warnings.push(format!(
                "{block} (fresh install: setup will configure the dataplane ports)"
            ));
        } else {
            blocks.push(block);
        }
    }

    if !compile.is_pass() {
        blocks.extend(compile.errors.iter().cloned());
    }
    warnings.extend(compile.warnings.iter().cloned());

    match circuit_count {
        Some(count) if count as u64 > circuit_limit => {
            // The raw row count is a LOWER BOUND on the mapped-circuit count,
 // so this can only over-warn — the safe direction.
            warnings.push(format!(
                "at least {count} circuits; the unlicensed cap is {circuit_limit} — \
                 v3 leaves the beyond-cap circuits unshaped until a license is installed \
                 (the mapped count computes at first boot)"
            ));
        }
        Some(_) => {}
        None => {
            if !warnings.iter().any(|warning| warning.contains("no topology source yet")) {
                warnings.push(NO_TOPOLOGY_SOURCE.to_string());
            }
        }
    }

    let severity = if !blocks.is_empty() {
        Severity::Block
    } else if !warnings.is_empty() {
        Severity::Warn
    } else {
        Severity::Pass
    };

    Verdict {
        severity,
        compile,
        circuit_count,
        circuit_limit,
        warnings,
        blocks,
        hardware,
        os: None,
        netplan: None,
        nic_plan: None,
        fetch: None,
        repo: None,
        seam: None,
        install: None,
        units: inventory(units_probe),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new() -> Self {
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "lqos_preflight_verdict_{}_{}",
                std::process::id(),
                id
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn write(&self, name: &str, contents: &str) {
            std::fs::write(self.0.join(name), contents).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const HEADER: &str = "Circuit ID,Circuit Name,Device ID,Device Name,Parent Node,Sparking Min,Sparking Max,IPv4,IPv6,Download Min,Upload Min,Download Max,Upload Max\n";

    fn csv_with_rows(rows: usize) -> String {
        let mut out = String::from(HEADER);
        for i in 0..rows {
            out.push_str(&format!(
                "{i},Circuit {i},{i},Device {i},, , ,192.168.1.{},,25,5,10000,10000\n",
                (i % 200) + 1
            ));
        }
        out
    }

    #[test]
    fn a_fresh_install_without_a_pair_warns_and_never_blocks() {
        // lqos-74t.37.10: a clean 24.04 host has no legacy pair yet. The
        // preinst must not refuse the package; setup creates the topology.
        let dir = Scratch::new();
        let verdict = compose(&dir.0, HardwareVerdict::all_ok(), |_| UnitPresence::Absent);
        assert_eq!(verdict.severity, Severity::Warn);
        assert!(verdict.blocks.is_empty(), "a fresh install must never BLOCK");
        assert!(
            verdict
                .warnings
                .iter()
                .any(|warning| warning.contains("no topology source yet")),
            "{:?}",
            verdict.warnings
        );
    }

    #[test]
    fn a_fresh_install_downgrades_the_hardware_pair_block() {
        use crate::hardware::HardwareClass;
        let dir = Scratch::new();
        let mut hardware = HardwareVerdict::all_ok();
        hardware.class = HardwareClass::Blocked;
        hardware.blocks.push(
            "inline deployment needs a PAIR of dataplane NICs; found 0".to_string(),
        );
        let fresh = compose_with_fresh(
            &dir.0,
            hardware.clone(),
            |_| UnitPresence::Absent,
            true,
        );
        assert_eq!(fresh.severity, Severity::Warn);
        assert!(fresh.blocks.is_empty(), "fresh install must not block on the pair");
        assert!(
            fresh
                .warnings
                .iter()
                .any(|warning| warning.contains("fresh install")),
            "{:?}",
            fresh.warnings
        );
        // An upgrade (fresh=false) keeps the strict pair block.
        let upgrade = compose(&dir.0, hardware.clone(), |_| UnitPresence::Absent);
        assert_eq!(upgrade.severity, Severity::Block);
    }

    #[test]
    fn a_half_legacy_pair_warns_and_never_blocks() {
        let dir = Scratch::new();
        dir.write("network.json", "{}"); // no ShapedDevices.csv yet
        let verdict = compose(&dir.0, HardwareVerdict::all_ok(), |_| UnitPresence::Absent);
        assert_eq!(verdict.severity, Severity::Warn);
        assert!(verdict.blocks.is_empty());
        assert!(
            verdict
                .warnings
                .iter()
                .any(|warning| warning.contains("incomplete manual topology")),
            "{:?}",
            verdict.warnings
        );
    }

    #[test]
    fn a_present_but_broken_source_blocks() {
        let dir = Scratch::new();
        dir.write("network.json", "this is not json");
        dir.write("ShapedDevices.csv", &csv_with_rows(1));
        let verdict = compose(&dir.0, HardwareVerdict::all_ok(), |_| UnitPresence::Absent);
        assert_eq!(verdict.severity, Severity::Block);
        assert!(!verdict.blocks.is_empty());
    }

    #[test]
    fn a_small_valid_pair_passes() {
        let dir = Scratch::new();
        dir.write("network.json", "{}");
        dir.write("ShapedDevices.csv", &csv_with_rows(2));
        let verdict = compose(&dir.0, HardwareVerdict::all_ok(), |_| UnitPresence::Absent);
        assert_eq!(verdict.severity, Severity::Pass);
        assert!(verdict.blocks.is_empty());
        assert_eq!(verdict.circuit_count, Some(2));
    }

    /// #5(A) NOTE (DRIVER ruling): the install-integrity disposition is a WARN
    /// ("unsupported"), NEVER a Block — a hand-rolled box that otherwise
    /// validates KEEPS shaping (card §B3). Pins the severity so a future edit
    /// cannot turn the report into a refuse.
    #[test]
    fn an_unsupported_install_integrity_is_a_warn_never_a_block() {
        let dir = Scratch::new();
        dir.write("network.json", "{}");
        dir.write("ShapedDevices.csv", &csv_with_rows(2));
        let report = InstallIntegrityReport {
            package_owned: false,
            defined_set_intact: None,
            package: None,
        };
        let verdict = compose(&dir.0, HardwareVerdict::all_ok(), |_| UnitPresence::Absent)
            .with_install(report);
        assert_eq!(
            verdict.severity,
            Severity::Warn,
            "a hand-rolled tree => WARN (keeps shaping), never Block"
        );
        assert!(verdict.blocks.is_empty(), "the install check never blocks");
        assert!(
            verdict
                .warnings
                .iter()
                .any(|warning| warning.contains("NOT package-owned")),
            "the named install warning is surfaced"
        );
    }

    #[test]
    fn an_over_cap_pair_warns_with_the_lower_bound_label() {
        let dir = Scratch::new();
        dir.write("network.json", "{}");
        dir.write(
            "ShapedDevices.csv",
            &csv_with_rows(DEFAULT_MAPPED_CIRCUIT_LIMIT as usize + 1),
        );
        let verdict = compose(&dir.0, HardwareVerdict::all_ok(), |_| UnitPresence::Absent);
        assert_eq!(verdict.severity, Severity::Warn);
        assert!(verdict.warnings[0].contains("at least 1001 circuits"));
    }

    #[test]
    fn a_broken_present_source_blocks() {
        let dir = Scratch::new();
        dir.write("network.json", "this is not json");
        dir.write("ShapedDevices.csv", &csv_with_rows(1));
        let verdict = compose(&dir.0, HardwareVerdict::all_ok(), |_| UnitPresence::Absent);
        assert_eq!(verdict.severity, Severity::Block);
        assert!(!verdict.blocks.is_empty());
    }

    #[test]
    fn a_pre_noble_os_blocks_with_the_plain_message() {
        let dir = Scratch::new();
        dir.write("network.json", "{}");
        dir.write("ShapedDevices.csv", &csv_with_rows(1));
        let release = os::parse_os_release(
            "PRETTY_NAME=\"Ubuntu 22.04.5 LTS\"\nID=ubuntu\nVERSION_ID=\"22.04\"\n",
        );
        let verdict =
            compose(&dir.0, HardwareVerdict::all_ok(), |_| UnitPresence::Absent).with_os(release);
        assert_eq!(verdict.severity, Severity::Block);
        assert!(
            verdict
                .blocks
                .iter()
                .any(|b| b.starts_with("Ubuntu 24.04+ required"))
        );
    }

    #[test]
    fn a_noble_os_does_not_change_a_passing_verdict() {
        let dir = Scratch::new();
        dir.write("network.json", "{}");
        dir.write("ShapedDevices.csv", &csv_with_rows(1));
        let release = os::parse_os_release(
            "PRETTY_NAME=\"Ubuntu 24.04.4 LTS\"\nID=ubuntu\nVERSION_ID=\"24.04\"\n",
        );
        let verdict =
            compose(&dir.0, HardwareVerdict::all_ok(), |_| UnitPresence::Absent).with_os(release);
        assert_eq!(verdict.severity, Severity::Pass);
        assert!(verdict.os.is_some());
    }

    #[test]
    fn a_netplan_bridge_over_the_pair_warns() {
        let dir = Scratch::new();
        dir.write("network.json", "{}");
        dir.write("ShapedDevices.csv", &csv_with_rows(1));
        let report = NetplanReport {
            pair: vec!["enp1s0f0".to_string(), "enp1s0f1".to_string()],
            bridges: Vec::new(),
            conflicting: vec![crate::netplan::NetplanBridge {
                name: "br0".to_string(),
                file: "/etc/netplan/50-cloud-init.yaml".into(),
                members: vec!["enp1s0f0".to_string(), "enp1s0f1".to_string()],
                management_at_risk: false,
            }],
            management_at_risk: false,
        };
        let verdict = compose(&dir.0, HardwareVerdict::all_ok(), |_| UnitPresence::Absent)
            .with_netplan(report);
        assert_eq!(verdict.severity, Severity::Warn);
        assert!(verdict.warnings.iter().any(|w| w.contains("br0")));
    }

    #[test]
    fn a_netplan_management_risk_gets_the_operator_console_prefix() {
        let dir = Scratch::new();
        dir.write("network.json", "{}");
        dir.write("ShapedDevices.csv", &csv_with_rows(1));
        let report = NetplanReport {
            pair: vec!["enp1s0f0".to_string()],
            bridges: Vec::new(),
            conflicting: vec![crate::netplan::NetplanBridge {
                name: "br0".to_string(),
                file: "/etc/netplan/50-cloud-init.yaml".into(),
                members: vec!["enp1s0f0".to_string()],
                management_at_risk: true,
            }],
            management_at_risk: true,
        };
        let verdict = compose(&dir.0, HardwareVerdict::all_ok(), |_| UnitPresence::Absent)
            .with_netplan(report);
        assert!(
            verdict
                .warnings
                .iter()
                .any(|w| w.contains("OPERATOR-CONSOLE REQUIRED"))
        );
    }

    #[test]
    fn the_units_census_is_informational_only() {
        let dir = Scratch::new();
        dir.write("network.json", "{}");
        dir.write("ShapedDevices.csv", &csv_with_rows(1));
        let verdict = compose(&dir.0, HardwareVerdict::all_ok(), |unit| {
            if unit == "lqosd.service" {
                UnitPresence::Active
            } else {
                UnitPresence::Absent
            }
        });
        assert_eq!(verdict.severity, Severity::Pass);
        assert_eq!(verdict.units.len(), crate::units::LEGACY_V2_UNITS.len());
    }
}
