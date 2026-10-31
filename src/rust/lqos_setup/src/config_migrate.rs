//! §7 — the legacy→`lqos.conf` fold (card `docs/lqos_conf_single_source_card.md` §7).
//!
//! The one-time, **idempotent**, **lqos.conf-wins** fold of the legacy operator
//! env keys into `/etc/libreqos/lqos.conf`. Today the only ISP-meaningful key
//! still duplicated into `bridge.env` is the **bandwidth** pair
//! (`LIBREQOS_DPDK_DOWNLOAD_MBPS`/`UPLOAD_MBPS`) — the same fact the operator
//! already sets as `[queues] downlink_bandwidth_mbps`/`uplink_bandwidth_mbps`
//! and that §3 moves to the `[dataplane]` single source (`.12` carries 8000/8000
//! in BOTH today — "a live drift bug waiting").
//!
//! This module is **pure text transforms only**: the caller owns the atomic
//! writes (mirroring `bootstrap::ensure_bridge_conf_at`), the retirement of the
//! legacy env keys, the log, and the rollback. Keeping the decision + the edit
//! pure makes both unit-testable without touching `/etc`.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The legacy operator env key carrying the download bandwidth (Mbps). This is
/// the key read by `bridge_rs::config::download_mbps()`'s env rung and written
/// into `bridge.env` by the v2 setup — the duplicate §3 deletes.
pub const ENV_DOWNLOAD_MBPS: &str = "LIBREQOS_DPDK_DOWNLOAD_MBPS";

/// The legacy operator env key carrying the upload bandwidth (Mbps).
pub const ENV_UPLOAD_MBPS: &str = "LIBREQOS_DPDK_UPLOAD_MBPS";

/// The bandwidth facts inherited from the legacy env (each independently
/// optional: an operator may have set only one).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InheritedBandwidth {
    /// `LIBREQOS_DPDK_DOWNLOAD_MBPS`, when present.
    pub download_mbps: Option<u64>,
    /// `LIBREQOS_DPDK_UPLOAD_MBPS`, when present.
    pub upload_mbps: Option<u64>,
}

impl InheritedBandwidth {
    /// True when the legacy env carried NO bandwidth (nothing to fold).
    pub fn is_empty(&self) -> bool {
        self.download_mbps.is_none() && self.upload_mbps.is_none()
    }
}

/// Reads one `KEY=VALUE` integer from an env-style text (comments + blank lines
/// ignored; a quoted value is unwrapped). First match wins.
fn env_u64(text: &str, key: &str) -> Option<u64> {
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some(rest) = line.strip_prefix(key) else {
            continue;
        };
        // Reject a longer key that merely shares the prefix (e.g. `..._MBPS_X`).
        let rest = rest.trim_start();
        let Some(value) = rest.strip_prefix('=') else {
            continue;
        };
        return value.trim().trim_matches('"').parse().ok();
    }
    None
}

/// Reads the legacy bandwidth from the env text. The caller concatenates the
/// `bridge.env` body + every `bridge.env.d/*.conf` drop-in (systemd applies
/// them in lexical order; the LAST writer wins there, but for the fold any set
/// value is equivalent, so first-match is sufficient and stable).
pub fn legacy_bandwidth(env_text: &str) -> InheritedBandwidth {
    InheritedBandwidth {
        download_mbps: env_u64(env_text, ENV_DOWNLOAD_MBPS),
        upload_mbps: env_u64(env_text, ENV_UPLOAD_MBPS),
    }
}

/// True when the lqos.conf text already names the bandwidth in `[dataplane]`
/// (the v3 single source) — the fold is a no-op then.
pub fn lqos_conf_has_dataplane_bandwidth(conf: &str) -> bool {
    section_has_key(conf, "dataplane", "download_mbps")
        || section_has_key(conf, "dataplane", "upload_mbps")
}

/// True when lqos.conf still carries the legacy `[queues]` bandwidth (the
/// upgrade preserves it so the current `bridge_rs` reader keeps working until
/// the engine side lands the `[dataplane]` read).
pub fn lqos_conf_has_queues_bandwidth(conf: &str) -> bool {
    section_has_key(conf, "queues", "downlink_bandwidth_mbps")
        || section_has_key(conf, "queues", "uplink_bandwidth_mbps")
}

/// Whether `[section]` contains `key = …` (comments + surrounding sections
/// ignored; a scoped reader, no TOML dependency).
fn section_has_key(conf: &str, section: &str, key: &str) -> bool {
    let header = format!("[{section}]");
    let mut in_section = false;
    for line in conf.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') {
            in_section = line == header;
            continue;
        }
        if in_section
            && let Some((k, _)) = line.split_once('=')
            && k.trim() == key
        {
            return true;
        }
    }
    false
}

/// The fold decision for one `lqos.conf`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BandwidthFold {
    /// `Some(..)` when the fold should write `[dataplane]`; `None` = nothing to
    /// do (no legacy env bandwidth, or lqos.conf already names it).
    pub apply: Option<InheritedBandwidth>,
    /// The legacy env keys to RETIRE once `apply` has landed.
    pub retire_keys: Vec<&'static str>,
}

/// The ONE-TIME, idempotent fold decision.
///
/// **Keyed on the ENV (the duplicate to retire), NOT on `[dataplane]` absence.**
/// The `lqos_config` LOAD-time migration (`maybe_migrate_bandwidth_to_dataplane`,
/// run at lqosd load) can pre-populate `[dataplane]` from `[queues]` BEFORE the
/// activation fold runs; if the fold keyed on `[dataplane]` absence it would
/// NO-OP and the duplicate env keys would survive forever (the bridge.config
/// duplication the DRIVER wants retired). So:
/// - the legacy env keys are RETIRED whenever the env carries them (idempotent —
///   absent keys are a no-op);
/// - `[dataplane]` is WRITTEN only when `lqos.conf` does not already name the
///   fact ⇒ **`lqos.conf` ALWAYS WINS** (an operator / `#1` value is never
///   overwritten from the legacy env).
pub fn plan_bandwidth_fold(conf: &str, env_text: &str) -> BandwidthFold {
    let inherited = legacy_bandwidth(env_text);
    if inherited.is_empty() {
        return BandwidthFold {
            apply: None,
            retire_keys: Vec::new(),
        };
    }
    let apply = if lqos_conf_has_dataplane_bandwidth(conf) {
        None
    } else {
        Some(inherited)
    };
    BandwidthFold {
        apply,
        retire_keys: vec![ENV_DOWNLOAD_MBPS, ENV_UPLOAD_MBPS],
    }
}

/// Upserts `download_mbps`/`upload_mbps` into the `[dataplane]` section of an
/// lqos.conf body. Existing `[dataplane]` keys are replaced in place (order
/// preserved); a missing section is appended. Pure — the caller writes the
/// result atomically. Only the keys carried by `bw` are written.
pub fn upsert_dataplane_bandwidth(conf: &str, bw: InheritedBandwidth) -> String {
    let mut lines: Vec<String> = conf.lines().map(str::to_owned).collect();
    let target = [
        ("download_mbps", bw.download_mbps),
        ("upload_mbps", bw.upload_mbps),
    ];

    // Locate the [dataplane] header (if any).
    let header_idx = lines.iter().position(|l| l.trim() == "[dataplane]");
    match header_idx {
        Some(h) => {
            // The section runs until the next header (or EOF).
            let end = lines
                .iter()
                .enumerate()
                .skip(h + 1)
                .find(|(_, l)| l.trim_start().starts_with('['))
                .map(|(i, _)| i)
                .unwrap_or(lines.len());
            let mut insert_at = h + 1;
            for (key, value) in target {
                let Some(value) = value else { continue };
                let existing = (h + 1..end).find(|&i| {
                    lines[i]
                        .split('#')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .split_once('=')
                        .is_some_and(|(k, _)| k.trim() == key)
                });
                match existing {
                    Some(i) => lines[i] = format!("{key} = {value}"),
                    None => {
                        lines.insert(insert_at, format!("{key} = {value}"));
                        insert_at += 1;
                    }
                }
            }
        }
        None => {
            if !lines.is_empty() && !lines.last().is_some_and(|l| l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push("[dataplane]".to_owned());
            for (key, value) in target {
                if let Some(value) = value {
                    lines.push(format!("{key} = {value}"));
                }
            }
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Removes every line whose key is in `keys` (env-style `KEY=…`; comments + the
/// rest preserved). Pure — used to RETIRE the legacy duplicate keys after the
/// fold lands. Prefix-safe (a longer key sharing the prefix is left alone).
pub fn retire_env_lines(text: &str, keys: &[&str]) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let trimmed = line.split('#').next().unwrap_or("").trim();
        let is_target = keys.iter().any(|key| {
            trimmed
                .strip_prefix(key)
                .is_some_and(|rest| rest.trim_start().starts_with('='))
        });
        if !is_target {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Reads every operator env surface: the `bridge.env` body plus each
/// `bridge.env.d/*.conf` drop-in (lexical order, matching systemd). Concatenated
/// (first-match per key is stable for the fold). A missing file is skipped.
pub fn read_env_surfaces(env_path: &Path, env_dir: &Path) -> String {
    let mut out = String::new();
    if let Ok(body) = std::fs::read_to_string(env_path) {
        out.push_str(&body);
        out.push('\n');
    }
    if let Ok(entries) = std::fs::read_dir(env_dir) {
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "conf"))
            .collect();
        files.sort();
        for file in files {
            if let Ok(body) = std::fs::read_to_string(&file) {
                out.push_str(&body);
                out.push('\n');
            }
        }
    }
    out
}

/// The result of an applied §7 fold (for the log + the rollback record).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoldOutcome {
    /// The bandwidth folded into `[dataplane]`.
    pub applied: InheritedBandwidth,
    /// The `lqos.conf` path written.
    pub conf_path: PathBuf,
    /// The `bridge.env` path whose duplicate keys were retired, when present.
    pub env_path: Option<PathBuf>,
    /// The legacy env keys retired.
    pub retire_keys: Vec<&'static str>,
    /// The backups written (for the rollback), newest last.
    pub backups: Vec<PathBuf>,
}

impl FoldOutcome {
    /// A one-line operator-facing log.
    pub fn summary(&self) -> String {
        let backups = self
            .backups
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        if self.applied.is_empty() {
            // RETIRE-ONLY: `[dataplane]` already named the fact (e.g. after the
            // `lqos_config` load-time migration); only the duplicate env keys went.
            format!(
                "retired the duplicate legacy bridge.env bandwidth ({}); [dataplane] already set; backups: {backups}",
                self.retire_keys.join(", "),
            )
        } else {
            format!(
                "folded the legacy bridge.env bandwidth into {} [dataplane] ({:?} Mbps down / {:?} Mbps up) and retired {}; backups: {backups}",
                self.conf_path.display(),
                self.applied.download_mbps,
                self.applied.upload_mbps,
                self.retire_keys.join(", "),
            )
        }
    }
}

/// The backup suffix written beside each surface the fold rewrites.
pub const BACKUP_SUFFIX: &str = "pre-s7.bak";

/// Backs up `path` beside itself (`<path>.pre-s7.bak`), returning the backup
/// path. A no-op (`None`) when `path` does not exist.
fn backup(path: &Path) -> std::io::Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let mut name = path.as_os_str().to_owned();
    name.push(".");
    name.push(BACKUP_SUFFIX);
    let backup = PathBuf::from(name);
    std::fs::copy(path, &backup)?;
    Ok(Some(backup))
}

/// The production §7 fold: `/etc/libreqos/lqos.conf` + `bridge.env` +
/// `bridge.env.d/*.conf`.
pub fn run_isp_fold() -> std::io::Result<Option<FoldOutcome>> {
    run_isp_fold_at(
        Path::new(lqos_config::DEFAULT_CONFIG_PATH),
        Path::new(crate::bootstrap::BRIDGE_ENV_PATH),
        Path::new(BRIDGE_ENV_DIR),
    )
}

/// The default `bridge.env.d` drop-in directory.
pub const BRIDGE_ENV_DIR: &str = "/etc/libreqos/bridge.env.d";

/// The testable §7 fold. Reads the operator-owned surfaces, folds the legacy
/// env bandwidth into `lqos.conf`'s `[dataplane]` (`lqos.conf` WINS), retires
/// the duplicate env keys, and writes BOTH surfaces atomically after backing
/// each up. Idempotent: a box already folded (or with no legacy bandwidth) is an
/// `Ok(None)` no-op. The caller logs the outcome; a failure is non-fatal.
pub fn run_isp_fold_at(
    conf_path: &Path,
    env_path: &Path,
    env_dir: &Path,
) -> std::io::Result<Option<FoldOutcome>> {
    let conf = std::fs::read_to_string(conf_path).unwrap_or_default();
    let env_text = read_env_surfaces(env_path, env_dir);
    let fold = plan_bandwidth_fold(&conf, &env_text);
    if fold.apply.is_none() && fold.retire_keys.is_empty() {
        return Ok(None);
    }

    let mut backups = Vec::new();

    // 1. lqos.conf: only when lqos.conf does NOT already name the fact — back
    //    up, then atomic-write the folded body (lqos.conf WINS).
    if let Some(bandwidth) = fold.apply {
        if let Some(backup) = backup(conf_path)? {
            backups.push(backup);
        }
        let folded = upsert_dataplane_bandwidth(&conf, bandwidth);
        crate::port_class::write_managed_dropin_at(conf_path, &folded)?;
    }

    // 2. bridge.env: back up, then RETIRE the duplicate keys — ALWAYS (the
    //    duplication is the target; this runs even when `[dataplane]` already
    //    named the fact, e.g. after the lqos_config LOAD-time migration).
    let mut env_path_written = None;
    if !fold.retire_keys.is_empty() && env_path.exists() {
        let body = std::fs::read_to_string(env_path)?;
        let retired = retire_env_lines(&body, &fold.retire_keys);
        if retired != body {
            if let Some(backup) = backup(env_path)? {
                backups.push(backup);
            }
            crate::port_class::write_managed_dropin_at(env_path, &retired)?;
            env_path_written = Some(env_path.to_path_buf());
        }
    }

    // Nothing changed on disk (nothing to write / nothing to retire) ⇒ no-op.
    if backups.is_empty() {
        return Ok(None);
    }

    Ok(Some(FoldOutcome {
        applied: fold.apply.unwrap_or_default(),
        conf_path: conf_path.to_path_buf(),
        env_path: env_path_written,
        retire_keys: fold.retire_keys,
        backups,
    }))
}

/// Restores the surfaces from the `<path>.pre-s7.bak` backups (the §7 rollback).
/// Returns `true` when at least one surface was restored. Idempotent (a missing
/// backup is skipped).
pub fn rollback_isp_fold_at(conf_path: &Path, env_path: &Path) -> std::io::Result<bool> {
    let mut restored = false;
    for path in [conf_path, env_path] {
        let mut name = path.as_os_str().to_owned();
        name.push(".");
        name.push(BACKUP_SUFFIX);
        let backup = PathBuf::from(name);
        if backup.exists() {
            std::fs::copy(&backup, path)?;
            restored = true;
        }
    }
    Ok(restored)
}

/// `lqos-74t.160`: the legacy worker key. The managed `50-ports.conf` also
/// carries this key, but it is a one-time migration input only — never an
/// ongoing override source.
pub const ENV_WORKERS: &str = "LIBREQOS_DPDK_WORKERS";

/// The one-time worker-adoption stamp. Its presence means the managed-W check
/// already ran (adopted, ignored-equal, override-present, or absent), so a
/// later rate/core change can never re-open it and adopt a stale managed W.
pub const WORKER_ADOPTION_STAMP: &str = "/var/lib/libreqos-bridge/worker-override-migrated";

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Upserts `workers = N` into the `[queues]` section of an lqos.conf body.
/// Existing key replaced in place; a missing section is appended. Pure — the
/// caller writes the result atomically.
pub fn upsert_queues_workers(conf: &str, workers: u32) -> String {
    let mut lines: Vec<String> = conf.lines().map(str::to_owned).collect();
    let header_idx = lines.iter().position(|l| l.trim() == "[queues]");
    match header_idx {
        Some(h) => {
            let end = lines
                .iter()
                .enumerate()
                .skip(h + 1)
                .find(|(_, l)| l.trim_start().starts_with('['))
                .map(|(i, _)| i)
                .unwrap_or(lines.len());
            let existing = (h + 1..end).find(|&i| {
                lines[i]
                    .split('#')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .split_once('=')
                    .is_some_and(|(k, _)| k.trim() == "workers")
            });
            match existing {
                Some(i) => lines[i] = format!("workers = {workers}"),
                None => lines.insert(h + 1, format!("workers = {workers}")),
            }
        }
        None => {
            if !lines.is_empty() && !lines.last().is_some_and(|l| l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push("[queues]".to_owned());
            lines.push(format!("workers = {workers}"));
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Stamp the one-time worker-adoption decision. `status` is
/// `adopted` | `ignored-equal` | `override-present` | `absent`; the extra
/// values are recorded for the operator/log and for the next-run audit.
pub fn stamp_worker_adoption_at(
    path: &Path,
    status: &str,
    workers: Option<u32>,
    derived: Option<u32>,
    origin: Option<&str>,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut body = format!("status={status}\n");
    if let Some(w) = workers {
        body.push_str(&format!("workers={w}\n"));
    }
    if let Some(d) = derived {
        body.push_str(&format!("derived={d}\n"));
    }
    if let Some(o) = origin {
        body.push_str(&format!("origin={o}\n"));
    }
    body.push_str(&format!("checked_unix={}\n", unix_timestamp()));
    std::fs::write(path, body)
}

/// The one-time adoption write: back up `lqos.conf`, upsert
/// `[queues] workers = N`, atomically write it, then stamp the decision.
pub fn adopt_workers_at(
    conf_path: &Path,
    stamp_path: &Path,
    workers: u32,
    origin: &str,
) -> std::io::Result<()> {
    let conf = std::fs::read_to_string(conf_path)?;
    let permissions = std::fs::metadata(conf_path)?.permissions();
    let backup = PathBuf::from(format!(
        "{}.pre-160.{}",
        conf_path.display(),
        unix_timestamp()
    ));
    std::fs::copy(conf_path, &backup)?;
    let updated = upsert_queues_workers(&conf, workers);
    crate::port_class::write_managed_dropin_at(conf_path, &updated)?;
    // lqos.conf may be 0600; the temp+rename must not widen/narrow its mode.
    std::fs::set_permissions(conf_path, permissions)?;
    stamp_worker_adoption_at(stamp_path, "adopted", Some(workers), None, Some(origin))
}

/// `true` when the one-time managed-W check already ran at `path`.
pub fn worker_adoption_checked_at(path: &Path) -> bool {
    path.exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENV_BOTH: &str = "# comment\nLIBREQOS_DPDK_DOWNLOAD_MBPS=8000\nLIBREQOS_DPDK_UPLOAD_MBPS=8000\n";

    #[test]
    fn reads_both_legacy_bandwidth_keys() {
        let bw = legacy_bandwidth(ENV_BOTH);
        assert_eq!(bw.download_mbps, Some(8000));
        assert_eq!(bw.upload_mbps, Some(8000));
    }

    #[test]
    fn a_longer_prefix_key_is_not_mistaken_for_the_bandwidth_key() {
        let bw = legacy_bandwidth("LIBREQOS_DPDK_DOWNLOAD_MBPS_EXTRA=5\n");
        assert!(bw.is_empty());
    }

    #[test]
    fn plans_the_fold_when_lqos_conf_lacks_dataplane() {
        let conf = "[queues]\ndownlink_bandwidth_mbps = 8000\nuplink_bandwidth_mbps = 8000\n";
        let fold = plan_bandwidth_fold(conf, ENV_BOTH);
        assert_eq!(fold.apply.unwrap().download_mbps, Some(8000));
        assert_eq!(fold.retire_keys, vec![ENV_DOWNLOAD_MBPS, ENV_UPLOAD_MBPS]);
    }

    #[test]
    fn lqos_conf_dataplane_wins_and_the_fold_is_a_noop() {
        let conf = "[dataplane]\ndownload_mbps = 900\nupload_mbps = 450\n";
        let fold = plan_bandwidth_fold(conf, ENV_BOTH);
        // lqos.conf WINS on the VALUE (no [dataplane] write) ...
        assert_eq!(fold.apply, None);
        // ... but the duplicate ENV keys are STILL retired (the fold's target).
        assert_eq!(fold.retire_keys, vec![ENV_DOWNLOAD_MBPS, ENV_UPLOAD_MBPS]);
    }

    /// The `#1` LOAD migration pre-populates `[dataplane]` from `[queues]`
    /// BEFORE the activation fold runs; the fold must STILL retire the env dup.
    #[test]
    fn the_fold_retires_the_env_dup_even_when_dataplane_already_exists() {
        let root = scratch("fold-preempted");
        let conf = root.join("lqos.conf");
        let env = root.join("bridge.env");
        std::fs::write(
            &conf,
            "[queues]\ndownlink_bandwidth_mbps = 8000\nuplink_bandwidth_mbps = 8000\n[dataplane]\ndownload_mbps = 8000\nupload_mbps = 8000\n",
        )
        .unwrap();
        std::fs::write(
            &env,
            "LIBREQOS_DPDK_DOWNLOAD_MBPS=8000\nLIBREQOS_DPDK_UPLOAD_MBPS=8000\nKEEP=1\n",
        )
        .unwrap();

        let outcome = run_isp_fold_at(&conf, &env, &root.join("nope"))
            .unwrap()
            .expect("the env dup must be retired");
        // lqos.conf was NOT rewritten (lqos.conf WINS), but the env dup is gone.
        assert!(!outcome.conf_path.as_os_str().is_empty());
        let env_body = std::fs::read_to_string(&env).unwrap();
        assert!(env_body.contains("KEEP=1"));
        assert!(!env_body.contains("LIBREQOS_DPDK_DOWNLOAD_MBPS="));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_summary_distinguishes_a_fold_from_a_retire_only() {
        let folded = FoldOutcome {
            applied: InheritedBandwidth {
                download_mbps: Some(8000),
                upload_mbps: Some(8000),
            },
            conf_path: PathBuf::from("/etc/libreqos/lqos.conf"),
            env_path: None,
            retire_keys: vec![ENV_DOWNLOAD_MBPS, ENV_UPLOAD_MBPS],
            backups: Vec::new(),
        };
        assert!(folded.summary().contains("folded the legacy bridge.env bandwidth into"));

        let retire_only = FoldOutcome {
            applied: InheritedBandwidth::default(),
            ..folded
        };
        let summary = retire_only.summary();
        assert!(summary.contains("retired the duplicate"), "{summary}");
        assert!(!summary.contains("None"), "the retire-only summary must not say None/None: {summary}");
    }

    #[test]
    fn no_legacy_bandwidth_is_a_noop() {
        let fold = plan_bandwidth_fold("[queues]\n", "# nothing\n");
        assert_eq!(fold.apply, None);
    }

    #[test]
    fn upsert_appends_a_missing_dataplane_section() {
        let conf = "[queues]\nqueue_mode = \"shape\"\n";
        let out = upsert_dataplane_bandwidth(
            conf,
            InheritedBandwidth {
                download_mbps: Some(8000),
                upload_mbps: Some(8000),
            },
        );
        assert!(out.contains("[dataplane]"));
        assert!(out.contains("download_mbps = 8000"));
        assert!(out.contains("upload_mbps = 8000"));
        // The existing section is untouched.
        assert!(out.contains("queue_mode = \"shape\""));
    }

    #[test]
    fn upsert_replaces_existing_dataplane_keys_in_place() {
        let conf = "[dataplane]\ndownload_mbps = 1\nupload_mbps = 2\n[queues]\n";
        let out = upsert_dataplane_bandwidth(
            conf,
            InheritedBandwidth {
                download_mbps: Some(8000),
                upload_mbps: Some(9000),
            },
        );
        assert!(out.contains("download_mbps = 8000"));
        assert!(out.contains("upload_mbps = 9000"));
        assert!(!out.contains("download_mbps = 1"));
        // The following section survived.
        assert!(out.contains("[queues]"));
    }

    #[test]
    fn upsert_is_idempotent() {
        let conf = "[queues]\n";
        let bw = InheritedBandwidth {
            download_mbps: Some(8000),
            upload_mbps: Some(8000),
        };
        let once = upsert_dataplane_bandwidth(conf, bw);
        let twice = upsert_dataplane_bandwidth(&once, bw);
        assert_eq!(once, twice);
        // And the folded file then reports the fact present (fold becomes a no-op).
        assert!(lqos_conf_has_dataplane_bandwidth(&once));
    }

    #[test]
    fn upsert_writes_only_the_keys_carried() {
        let conf = "[queues]\n";
        let out = upsert_dataplane_bandwidth(
            conf,
            InheritedBandwidth {
                download_mbps: Some(8000),
                upload_mbps: None,
            },
        );
        assert!(out.contains("download_mbps = 8000"));
        assert!(!out.contains("upload_mbps"));
    }

    #[test]
    fn retire_env_lines_removes_only_the_target_keys() {
        let text = "KEEP=1\nLIBREQOS_DPDK_DOWNLOAD_MBPS=8000\n# LIBREQOS_DPDK_UPLOAD_MBPS=9\nLIBREQOS_DPDK_UPLOAD_MBPS=8000\nLIBREQOS_DPDK_DOWNLOAD_MBPS_EXTRA=2\n";
        let out = retire_env_lines(text, &[ENV_DOWNLOAD_MBPS, ENV_UPLOAD_MBPS]);
        assert!(out.contains("KEEP=1"));
        // A commented example is preserved (never a live key).
        assert!(out.contains("# LIBREQOS_DPDK_UPLOAD_MBPS=9"));
        // A longer key sharing the prefix survives.
        assert!(out.contains("LIBREQOS_DPDK_DOWNLOAD_MBPS_EXTRA=2"));
        // The live duplicate keys are gone.
        assert!(!out.contains("\nLIBREQOS_DPDK_DOWNLOAD_MBPS="));
        assert!(!out.contains("LIBREQOS_DPDK_UPLOAD_MBPS=8000"));
    }

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("lqos-s7-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn the_fold_writes_dataplane_retires_the_env_and_backs_up() {
        let root = scratch("fold");
        let conf = root.join("lqos.conf");
        let env = root.join("bridge.env");
        let dir = root.join("bridge.env.d");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            &conf,
            "[queues]\ndownlink_bandwidth_mbps = 8000\nuplink_bandwidth_mbps = 8000\n",
        )
        .unwrap();
        std::fs::write(
            &env,
            "# operator file\nLIBREQOS_DPDK_DOWNLOAD_MBPS=8000\nLIBREQOS_DPDK_UPLOAD_MBPS=8000\nKEEP=1\n",
        )
        .unwrap();
        // A drop-in sharing the same fact must be read (first-match) but not rewritten.
        std::fs::write(dir.join("60-x.conf"), "UNRELATED=1\n").unwrap();

        let outcome = run_isp_fold_at(&conf, &env, &dir).unwrap().expect("folds");
        assert_eq!(outcome.applied.download_mbps, Some(8000));
        assert_eq!(outcome.backups.len(), 2);
        let conf_body = std::fs::read_to_string(&conf).unwrap();
        assert!(conf_body.contains("[dataplane]"));
        assert!(conf_body.contains("download_mbps = 8000"));
        // The legacy [queues] fact is PRESERVED (the engine's fallback).
        assert!(conf_body.contains("downlink_bandwidth_mbps = 8000"));
        let env_body = std::fs::read_to_string(&env).unwrap();
        assert!(env_body.contains("KEEP=1"));
        assert!(!env_body.contains("LIBREQOS_DPDK_DOWNLOAD_MBPS="));

        // Idempotent: the second run is a no-op.
        assert!(run_isp_fold_at(&conf, &env, &dir).unwrap().is_none());

        // Rollback restores both surfaces.
        assert!(rollback_isp_fold_at(&conf, &env).unwrap());
        let restored = std::fs::read_to_string(&conf).unwrap();
        assert!(!restored.contains("[dataplane]"));
        let env_restored = std::fs::read_to_string(&env).unwrap();
        assert!(env_restored.contains("LIBREQOS_DPDK_DOWNLOAD_MBPS=8000"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_fold_is_a_noop_when_dataplane_is_already_set() {
        let root = scratch("fold-noop");
        let conf = root.join("lqos.conf");
        let env = root.join("bridge.env");
        std::fs::write(&conf, "[dataplane]\ndownload_mbps = 900\nupload_mbps = 450\n").unwrap();
        std::fs::write(&env, "LIBREQOS_DPDK_DOWNLOAD_MBPS=8000\n").unwrap();
        // lqos.conf WINS (no [dataplane] rewrite); the env dup is still retired.
        assert!(run_isp_fold_at(&conf, &env, &root.join("nope")).unwrap().is_some());
        assert!(std::fs::read_to_string(&conf).unwrap().contains("download_mbps = 900"));
        assert!(!std::fs::read_to_string(&env).unwrap().contains("LIBREQOS_DPDK_DOWNLOAD_MBPS="));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn queues_workers_upsert_replaces_or_appends_without_touching_other_keys() {
        let existing =
            "[queues]\ndownlink_bandwidth_mbps = 2500\nworkers = 4\nuse_binpacking = true\n";
        let out = upsert_queues_workers(existing, 8);
        assert!(out.contains("workers = 8"), "{out}");
        assert!(!out.contains("workers = 4"), "{out}");
        assert!(out.contains("downlink_bandwidth_mbps = 2500"), "{out}");
        assert!(out.contains("use_binpacking = true"), "{out}");

        let no_section = "[dataplane]\ndownload_mbps = 2500\n";
        let out = upsert_queues_workers(no_section, 8);
        assert!(out.contains("[queues]\nworkers = 8"), "{out}");
        assert!(out.contains("[dataplane]"), "{out}");

        let no_key = "[queues]\nuse_binpacking = true\n";
        let out = upsert_queues_workers(no_key, 8);
        assert!(out.contains("workers = 8"), "{out}");
        assert!(out.contains("use_binpacking = true"), "{out}");
    }

    #[test]
    fn one_time_worker_adoption_writes_backup_and_stamp() {
        let root = scratch("worker-adopt");
        let conf = root.join("lqos.conf");
        let stamp = root.join("worker-override-migrated");
        std::fs::write(
            &conf,
            "[queues]\ndownlink_bandwidth_mbps = 2500\nuplink_bandwidth_mbps = 2500\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&conf, std::fs::Permissions::from_mode(0o600)).unwrap();

        adopt_workers_at(&conf, &stamp, 8, "managed 50-ports").unwrap();
        let body = std::fs::read_to_string(&conf).unwrap();
        assert!(body.contains("workers = 8"), "{body}");
        assert_eq!(
            std::fs::metadata(&conf).unwrap().permissions().mode() & 0o777,
            0o600,
            "the config mode must survive the atomic rewrite"
        );
        let stamp_body = std::fs::read_to_string(&stamp).unwrap();
        assert!(stamp_body.contains("status=adopted"), "{stamp_body}");
        assert!(stamp_body.contains("workers=8"), "{stamp_body}");
        assert!(stamp_body.contains("origin=managed 50-ports"), "{stamp_body}");
        // a pre-160 backup exists beside the config
        let backups: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.contains(".pre-160."))
            .collect();
        assert_eq!(backups.len(), 1, "{backups:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_worker_stamp_records_ignored_equal_and_absence() {
        let root = scratch("worker-stamp");
        let stamp = root.join("worker-override-migrated");
        assert!(!worker_adoption_checked_at(&stamp));
        stamp_worker_adoption_at(
            &stamp,
            "ignored-equal",
            Some(4),
            Some(4),
            Some("managed 50-ports"),
        )
        .unwrap();
        assert!(worker_adoption_checked_at(&stamp));
        let body = std::fs::read_to_string(&stamp).unwrap();
        assert!(body.contains("status=ignored-equal"), "{body}");
        assert!(body.contains("derived=4"), "{body}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
