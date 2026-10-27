//! Apply / restore / conflict detection (`docs/power_profile_work_order.md`
//! §4). Reversibility: the pre-existing values are recorded ONCE (first touch)
//! in `/var/lib/libreqos/power.orig`; every write is logged old→new; a VERIFY
//! step re-reads each written path and reports anything that did not stick.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::plan::{Facet, Setting, plan};
use crate::platform::{Platform, TurboOff};
use crate::profile::PowerProfile;
use crate::sysfs::{Commands, Sysfs};

/// The reversibility record: `sysfs path -> pre-existing value`.
pub const ORIGINALS_PATH: &str = "/var/lib/libreqos/power.orig";
/// The append-only change log.
pub const LOG_PATH: &str = "/var/lib/libreqos/power.log";

/// Services that can silently fight/revert the profile.
pub const CONFLICT_SERVICES: [&str; 3] = ["tuned", "cpupower", "power-profiles-daemon"];

/// Cumulative count of applies that ran while a conflicting service was active
/// (A2: a loud COUNTED warning, never warning-only).
static POWER_CONFLICTS: AtomicU64 = AtomicU64::new(0);

/// Cumulative conflict-observed applies (see [`POWER_CONFLICTS`]).
pub fn conflict_count() -> u64 {
    POWER_CONFLICTS.load(Ordering::Relaxed)
}

/// A requested apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApplyRequest {
    pub profile: PowerProfile,
    /// `[power] eco_max_freq_pct` (used only by `eco`).
    pub eco_max_freq_pct: u64,
    /// Proceed despite an active conflicting service (A2). `false` REFUSES.
    pub force: bool,
    pub dry_run: bool,
}

/// One performed (or planned) write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppliedWrite {
    pub facet: Facet,
    pub path: String,
    pub old: Option<String>,
    pub new: String,
}

/// The outcome of an `apply`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplyReport {
    pub profile: PowerProfile,
    pub virtualized: bool,
    pub dry_run: bool,
    pub forced: bool,
    /// Active conflicting services (named).
    pub conflicts: Vec<String>,
    /// How turbo-off is enforced on this host (hard knob vs nominal cap vs
    /// unsupported). Reported by the CLI and covered by tests.
    pub turbo_off: TurboOff,
    pub writes: Vec<AppliedWrite>,
    /// A2 VERIFY: `(path, expected, actual)` for writes that did not stick.
    pub unverified: Vec<(String, String, String)>,
    /// When virtualized: the exact host-side steps (never written from the
    /// guest).
    pub host_steps: Vec<String>,
}

impl ApplyReport {
    pub fn changed(&self) -> usize {
        self.writes
            .iter()
            .filter(|w| w.old.as_deref() != Some(w.new.as_str()))
            .count()
    }
}

/// Detect active control services that can fight the profile.
pub fn detect_conflicts(cmds: &impl Commands) -> Vec<String> {
    CONFLICT_SERVICES
        .iter()
        .filter(|svc| {
            cmds.run("systemctl", &["is-active", "--quiet", svc])
                .is_some_and(|(code, _)| code == 0)
        })
        .map(|s| (*s).to_string())
        .collect()
}

fn load_originals(path: &str) -> BTreeMap<String, String> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save_originals(path: &str, originals: &BTreeMap<String, String>) -> Result<()> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let raw = serde_json::to_string_pretty(originals)?;
    std::fs::write(path, raw).with_context(|| format!("write {path}"))?;
    Ok(())
}

fn append_log(line: &str) {
    use std::io::Write as _;
    tracing::info!("{line}");
    if let Some(parent) = std::path::Path::new(LOG_PATH).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(LOG_PATH)
    {
        let _ = writeln!(f, "{line}");
    }
}

/// Apply the request via the seams.
///
/// * **Conflicts (A2):** an active `tuned`/`cpupower`/`power-profiles-daemon`
///   **REFUSES** unless `force` is set (they rewrite sysfs after the record, so
///   the recorded originals would be stale). A forced run counts + warns
///   loudly.
/// * **Bare metal:** plan → record originals (first touch) → write each (logged
///   old→new) → VERIFY (re-read; report non-sticks).
/// * **VM:** NEVER write; return the exact host steps.
pub fn apply(
    request: &ApplyRequest,
    sysfs: &impl Sysfs,
    cmds: &impl Commands,
    originals_path: &str,
) -> Result<ApplyReport> {
    let platform = Platform::detect(sysfs, cmds);
    let turbo_off = platform.turbo_off();
    let conflicts = detect_conflicts(cmds);

    if platform.virtualized {
        return Ok(ApplyReport {
            profile: request.profile,
            virtualized: true,
            dry_run: request.dry_run,
            forced: request.force,
            conflicts,
            turbo_off,
            writes: Vec::new(),
            unverified: Vec::new(),
            host_steps: crate::plan::host_steps(request.profile),
        });
    }

    if !conflicts.is_empty() {
        let total = POWER_CONFLICTS.fetch_add(1, Ordering::Relaxed) + 1;
        if !request.force {
            bail!(
                "refusing to apply the power profile: {} is active and will revert/fight it \
                 (it rewrites sysfs after the reversibility record). Stop it, or pass --force to \
                 proceed anyway (counted as conflict #{total}).",
                conflicts.join(", ")
            );
        }
        tracing::warn!(
            total,
            conflicts = ?conflicts,
            "power profile applied DESPITE an active control service (--force); the original \
             values were recorded BEFORE it may rewrite them"
        );
    }

    let settings: Vec<Setting> = plan(request.profile, &platform, sysfs, request.eco_max_freq_pct);
    let mut originals = load_originals(originals_path);
    let mut writes = Vec::with_capacity(settings.len());
    let mut unverified = Vec::new();

    for setting in settings {
        // First-touch wins (A2): the ORIGINAL value is recorded once, never
        // overwritten by a later profile's pre-write value.
        if !request.dry_run && !originals.contains_key(&setting.path) {
            if let Some(old) = sysfs.read(&setting.path) {
                originals.insert(setting.path.clone(), old);
            }
        }
        let old = sysfs.read(&setting.path);
        if !request.dry_run {
            sysfs
                .write(&setting.path, &setting.value)
                .with_context(|| format!("write {} = {}", setting.path, setting.value))?;
            if old.as_deref() != Some(setting.value.as_str()) {
                append_log(&format!(
                    "power[{}]: {} {} -> {}",
                    request.profile,
                    setting.path,
                    old.as_deref().unwrap_or("<absent>"),
                    setting.value
                ));
            }
            // A2 VERIFY: the value must have stuck (a conflicting daemon or
            // an asynchronously-applied cpufreq write may need a short settle;
            // retry once and then report the final read).
            let mut actual = sysfs.read(&setting.path);
            if actual.as_deref() != Some(setting.value.as_str()) {
                std::thread::sleep(Duration::from_millis(25));
                let _ = sysfs.write(&setting.path, &setting.value);
                actual = sysfs.read(&setting.path);
            }
            if actual.as_deref() != Some(setting.value.as_str()) {
                unverified.push((
                    setting.path.clone(),
                    setting.value.clone(),
                    actual.unwrap_or_else(|| "<absent>".to_string()),
                ));
            }
        }
        writes.push(AppliedWrite {
            facet: setting.facet,
            path: setting.path,
            old,
            new: setting.value,
        });
    }

    if !request.dry_run {
        save_originals(originals_path, &originals)?;
    }

    Ok(ApplyReport {
        profile: request.profile,
        virtualized: false,
        dry_run: request.dry_run,
        forced: request.force,
        conflicts,
        turbo_off,
        writes,
        unverified,
        host_steps: Vec::new(),
    })
}

/// Restore every recorded pre-existing value and clear the record.
///
/// NOTE (A1): this is a FULL restore — if turbo was ON before the first apply,
/// restore RE-ENABLES it. The "turbo always off" gate is the PRODUCT's managed
/// default, not a block on an explicit operator restore (DRIVER ruling pending).
pub fn restore(sysfs: &impl Sysfs, originals_path: &str) -> Result<usize> {
    let originals = load_originals(originals_path);
    for (path, value) in &originals {
        let old = sysfs.read(path);
        sysfs
            .write(path, value)
            .with_context(|| format!("restore {path} = {value}"))?;
        append_log(&format!(
            "power[restore]: {} {} -> {}",
            path,
            old.as_deref().unwrap_or("<absent>"),
            value
        ));
    }
    if std::path::Path::new(originals_path).exists() {
        std::fs::remove_file(originals_path).with_context(|| format!("remove {originals_path}"))?;
    }
    Ok(originals.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sysfs::{MemorySysfs, NoCommands};

    fn req(profile: PowerProfile, force: bool, dry_run: bool) -> ApplyRequest {
        ApplyRequest {
            profile,
            eco_max_freq_pct: 65,
            force,
            dry_run,
        }
    }

    #[test]
    fn vm_never_writes_and_returns_host_steps() {
        struct VmCommands;
        impl Commands for VmCommands {
            fn run(&self, prog: &str, args: &[&str]) -> Option<(i32, String)> {
                (prog == "systemd-detect-virt" && args.contains(&"--quiet"))
                    .then_some((0, String::new()))
            }
        }
        let fs = MemorySysfs::new();
        let report = apply(
            &req(PowerProfile::Balanced, false, false),
            &fs,
            &VmCommands,
            "/tmp/opencode/none-power.orig",
        )
        .unwrap();
        assert!(report.virtualized);
        assert!(report.writes.is_empty(), "a VM writes nothing");
        assert!(!report.host_steps.is_empty(), "the host steps are printed");
    }

    #[test]
    fn active_conflict_refuses_unless_forced() {
        struct ConflictCommands;
        impl Commands for ConflictCommands {
            fn run(&self, prog: &str, args: &[&str]) -> Option<(i32, String)> {
                if prog == "systemctl"
                    && args.first() == Some(&"is-active")
                    && args.contains(&"tuned")
                {
                    return Some((0, String::new()));
                }
                if prog == "systemd-detect-virt" {
                    return Some((1, String::new())); // bare metal
                }
                Some((1, String::new()))
            }
        }
        let mut fs = MemorySysfs::new();
        fs.set("/sys/devices/system/cpu/intel_pstate/no_turbo", "0");
        fs.dir("/sys/devices/system/cpu/intel_pstate", &["no_turbo"]);
        fs.dir("/sys/devices/system/cpu", &["intel_pstate"]);
        let originals = "/tmp/opencode/power-conflict-test.orig";
        let _ = std::fs::remove_file(originals);

        assert!(
            apply(
                &req(PowerProfile::Balanced, false, false),
                &fs,
                &ConflictCommands,
                originals
            )
            .is_err(),
            "an active conflict REFUSES without --force"
        );
        let report = apply(
            &req(PowerProfile::Balanced, true, true),
            &fs,
            &ConflictCommands,
            originals,
        )
        .expect("--force proceeds");
        assert_eq!(report.conflicts, vec!["tuned".to_string()]);
        assert!(report.forced);
        let _ = std::fs::remove_file(originals);
    }

    #[test]
    fn apply_records_first_touch_originals() {
        let mut fs = MemorySysfs::new();
        fs.set("/sys/devices/system/cpu/intel_pstate/no_turbo", "0");
        fs.dir("/sys/devices/system/cpu/intel_pstate", &["no_turbo"]);
        fs.dir("/sys/devices/system/cpu", &["intel_pstate"]);
        let originals = "/tmp/opencode/power-test.orig";
        let _ = std::fs::remove_file(originals);

        let report = apply(
            &req(PowerProfile::Balanced, false, true),
            &fs,
            &NoCommands,
            originals,
        )
        .unwrap();
        assert_eq!(report.writes.len(), 1);
        assert_eq!(report.writes[0].new, "1");
        let _ = std::fs::remove_file(originals);
    }
}
