use crate::inspect::{
    NetworkModeInspection, adoption_rewrite_for_path, inspect_network_mode_with_paths,
};
use crate::protocol::{
    ApplyMode, ApplyRequest, ApplyResponse, BackupSummary, HelperStatus, PendingOperationStatus,
};
use anyhow::{Context, Result, anyhow, bail};
use lqos_config::Config;
use lqos_utils::unix_time::unix_now;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tracing::{info, warn};
use uuid::Uuid;

const BACKUP_KEEP_COUNT: usize = 5;

/// Controls whether helper actions should trigger a LibreQoS shaping reload.
#[derive(Clone, Debug)]
pub enum RetryShapingAction {
    LoadLibreQoS,
    None,
}

/// Filesystem and command paths used by the helper transaction engine.
#[derive(Clone, Debug)]
pub struct HelperPaths {
    pub config_path: PathBuf,
    pub netplan_dir: PathBuf,
    pub managed_netplan_path: PathBuf,
    pub backup_dir: PathBuf,
    pub pending_dir: PathBuf,
    pub netplan_bin: PathBuf,
    pub netplan_timeout_secs: u32,
    pub retry_shaping: RetryShapingAction,
}

impl Default for HelperPaths {
    fn default() -> Self {
        Self {
            config_path: PathBuf::from("/etc/lqos.conf"),
            netplan_dir: PathBuf::from("/etc/netplan"),
            managed_netplan_path: PathBuf::from("/etc/netplan/libreqos.yaml"),
            backup_dir: PathBuf::from("/var/lib/libreqos/netplan-backups"),
            pending_dir: PathBuf::from("/var/lib/libreqos/netplan-pending"),
            netplan_bin: PathBuf::from("/usr/sbin/netplan"),
            netplan_timeout_secs: 30,
            retry_shaping: RetryShapingAction::LoadLibreQoS,
        }
    }
}

/// Metadata written alongside each rollback bundle.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackupManifest {
    pub backup_id: String,
    pub timestamp_unix: u64,
    pub source: String,
    pub operator_username: Option<String>,
    pub pending_operation_id: Option<String>,
    pub old_mode: String,
    pub new_mode: String,
    #[serde(default)]
    pub old_interfaces: Vec<String>,
    #[serde(default)]
    pub new_interfaces: Vec<String>,
    pub takeover: bool,
    pub adoption: bool,
    #[serde(default)]
    pub files_touched: Vec<String>,
    #[serde(default)]
    pub warnings_present: Vec<String>,
}

/// Persisted state for a pending network change awaiting LibreQoS confirmation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingOperationRecord {
    pub operation_id: String,
    pub backup_id: String,
    pub source: String,
    pub operator_username: Option<String>,
    pub created_unix: u64,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
}

/// In-memory helper state. Pending operations are persisted on disk.
#[derive(Debug, Default)]
pub struct PendingChildren {
    scheduled_rollbacks: BTreeSet<String>,
}

fn now_unix() -> Result<u64> {
    Ok(unix_now()?)
}

fn mode_label(config: &Config) -> String {
    if let Some(bridge) = &config.bridge {
        if bridge.use_xdp_bridge {
            "XDP Bridge".to_string()
        } else {
            "Linux Bridge".to_string()
        }
    } else if config.single_interface.is_some() {
        "Single Interface".to_string()
    } else {
        "Unconfigured".to_string()
    }
}

fn mode_interfaces(config: &Config) -> Vec<String> {
    if let Some(bridge) = &config.bridge {
        [bridge.to_internet.trim(), bridge.to_network.trim()]
            .into_iter()
            .filter(|iface| !iface.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    } else if let Some(single) = &config.single_interface {
        if single.interface.trim().is_empty() {
            Vec::new()
        } else {
            vec![single.interface.trim().to_string()]
        }
    } else {
        Vec::new()
    }
}

fn load_config_from_path(path: &Path) -> Result<Config> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("Unable to read config at {}", path.display()))?;
    toml::from_str::<Config>(&raw)
        .with_context(|| format!("Unable to parse config at {}", path.display()))
}

fn write_config_to_path(path: &Path, config: &Config) -> Result<()> {
    let serialized = toml::to_string_pretty(config).context("Unable to serialize config")?;
    fs::write(path, serialized).with_context(|| format!("Unable to write {}", path.display()))
}

fn ensure_parent(path: &Path) -> Result<()> {
    let Some(parent) = path.parent() else {
        bail!("{} does not have a parent directory", path.display());
    };
    fs::create_dir_all(parent).with_context(|| format!("Unable to create {}", parent.display()))
}

fn write_text_file(path: &Path, body: &str) -> Result<()> {
    ensure_parent(path)?;
    fs::write(path, body).with_context(|| format!("Unable to write {}", path.display()))
}

fn write_netplan_file(path: &Path, body: &str) -> Result<()> {
    write_text_file(path, body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("Unable to chmod 600 {}", path.display()))?;
    }
    Ok(())
}

fn system_interfaces() -> BTreeSet<String> {
    default_net::get_interfaces()
        .into_iter()
        .map(|iface| iface.name)
        .collect()
}

fn supports_multi_queue(interface: &str) -> bool {
    let path = Path::new("/sys/class/net").join(interface).join("queues");
    let Ok(entries) = fs::read_dir(path) else {
        return false;
    };
    let mut rx = 0usize;
    let mut tx = 0usize;
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
            continue;
        };
        if name.starts_with("rx-") {
            rx += 1;
        } else if name.starts_with("tx-") {
            tx += 1;
        }
    }
    rx > 1 && tx > 1
}

/// Inspect the requested network mode using the helper's configured paths.
pub fn inspect_with_paths(paths: &HelperPaths, config: &Config) -> NetworkModeInspection {
    let system_ifaces = system_interfaces();
    let queue_caps = system_ifaces
        .iter()
        .map(|iface| (iface.clone(), supports_multi_queue(iface)))
        .collect::<BTreeMap<_, _>>();
    inspect_network_mode_with_paths(
        config,
        &paths.netplan_dir,
        &paths.pending_dir,
        &system_ifaces,
        &queue_caps,
    )
}

fn backup_manifest_path(paths: &HelperPaths, backup_id: &str) -> PathBuf {
    paths.backup_dir.join(backup_id).join("manifest.json")
}

fn pending_record_path(paths: &HelperPaths, operation_id: &str) -> PathBuf {
    paths.pending_dir.join(format!("{operation_id}.json"))
}

fn read_backup_manifest(paths: &HelperPaths, backup_id: &str) -> Result<BackupManifest> {
    let path = backup_manifest_path(paths, backup_id);
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("Unable to read backup manifest {}", path.display()))?;
    serde_json::from_str(&raw).context("Unable to parse backup manifest")
}

fn backup_summary(manifest: BackupManifest) -> BackupSummary {
    BackupSummary {
        backup_id: manifest.backup_id,
        timestamp_unix: manifest.timestamp_unix,
        source: manifest.source,
        operator_username: manifest.operator_username,
        old_mode: manifest.old_mode,
        new_mode: manifest.new_mode,
        warnings_present: manifest.warnings_present,
    }
}

fn read_pending_record(paths: &HelperPaths, operation_id: &str) -> Result<PendingOperationRecord> {
    let path = pending_record_path(paths, operation_id);
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("Unable to read pending operation {}", path.display()))?;
    serde_json::from_str(&raw).context("Unable to parse pending operation")
}

fn list_backup_ids(paths: &HelperPaths) -> Result<Vec<String>> {
    let mut ids = Vec::new();
    let Ok(entries) = fs::read_dir(&paths.backup_dir) else {
        return Ok(ids);
    };
    for entry in entries.flatten() {
        if entry.path().is_dir()
            && let Some(name) = entry.file_name().to_str()
        {
            ids.push(name.to_string());
        }
    }
    ids.sort();
    ids.reverse();
    Ok(ids)
}

fn list_recent_backups(paths: &HelperPaths) -> Result<Vec<BackupSummary>> {
    let mut backups = Vec::new();
    for backup_id in list_backup_ids(paths)?.into_iter().take(BACKUP_KEEP_COUNT) {
        let manifest = read_backup_manifest(paths, &backup_id)?;
        backups.push(backup_summary(manifest));
    }
    Ok(backups)
}

fn prune_backups(paths: &HelperPaths) -> Result<()> {
    let ids = list_backup_ids(paths)?;
    for stale in ids.into_iter().skip(BACKUP_KEEP_COUNT) {
        let path = paths.backup_dir.join(stale);
        fs::remove_dir_all(&path)
            .with_context(|| format!("Unable to remove old backup {}", path.display()))?;
    }
    Ok(())
}

fn write_backup_bundle(
    paths: &HelperPaths,
    previous_config: &Config,
    request: &ApplyRequest,
    inspection: &NetworkModeInspection,
    operation_id: &str,
    external_source_path: Option<&Path>,
) -> Result<String> {
    let backup_id = Uuid::new_v4().to_string();
    let backup_root = paths.backup_dir.join(&backup_id);
    fs::create_dir_all(&backup_root)
        .with_context(|| format!("Unable to create {}", backup_root.display()))?;

    let previous_config_path = backup_root.join("lqos.conf.before");
    if paths.config_path.exists() {
        write_config_to_path(&previous_config_path, previous_config)?;
    } else {
        write_text_file(&backup_root.join("lqos.conf.absent"), "absent\n")?;
    }

    let previous_managed_path = backup_root.join("libreqos.yaml.before");
    if paths.managed_netplan_path.exists() {
        fs::copy(&paths.managed_netplan_path, &previous_managed_path).with_context(|| {
            format!(
                "Unable to copy {} into backup bundle",
                paths.managed_netplan_path.display()
            )
        })?;
    } else {
        write_text_file(&backup_root.join("libreqos.yaml.absent"), "absent\n")?;
    }

    if let Some(source_path) = external_source_path {
        let source_backup_path = backup_root.join("adoption-source.before");
        fs::copy(source_path, &source_backup_path).with_context(|| {
            format!(
                "Unable to copy external adoption source {} into backup bundle",
                source_path.display()
            )
        })?;
        write_text_file(
            &backup_root.join("adoption-source.path"),
            &format!("{}\n", source_path.display()),
        )?;
    }

    let manifest = BackupManifest {
        backup_id: backup_id.clone(),
        timestamp_unix: now_unix()?,
        source: request.source.clone(),
        operator_username: request.operator_username.clone(),
        pending_operation_id: Some(operation_id.to_string()),
        old_mode: mode_label(previous_config),
        new_mode: mode_label(&request.config),
        old_interfaces: mode_interfaces(previous_config),
        new_interfaces: mode_interfaces(&request.config),
        takeover: request.mode == ApplyMode::TakeOver,
        adoption: request.mode == ApplyMode::Adopt,
        files_touched: {
            let mut files = vec![
                paths.config_path.display().to_string(),
                paths.managed_netplan_path.display().to_string(),
            ];
            if let Some(source_path) = external_source_path {
                files.push(source_path.display().to_string());
            }
            files
        },
        warnings_present: inspection
            .warnings
            .iter()
            .chain(inspection.dangerous_changes.iter())
            .cloned()
            .collect(),
    };

    let manifest_path = backup_root.join("manifest.json");
    let manifest_json =
        serde_json::to_string_pretty(&manifest).context("Unable to serialize backup manifest")?;
    write_text_file(&manifest_path, &manifest_json)?;
    prune_backups(paths)?;

    Ok(backup_id)
}

fn restore_backup_files(paths: &HelperPaths, backup_id: &str) -> Result<()> {
    let backup_root = paths.backup_dir.join(backup_id);
    let previous_config_path = backup_root.join("lqos.conf.before");
    let previous_config_absent = backup_root.join("lqos.conf.absent");
    let previous_managed_path = backup_root.join("libreqos.yaml.before");
    let absent_marker = backup_root.join("libreqos.yaml.absent");

    if previous_config_path.exists() {
        fs::copy(&previous_config_path, &paths.config_path).with_context(|| {
            format!(
                "Unable to restore {} from backup {}",
                paths.config_path.display(),
                previous_config_path.display()
            )
        })?;
    } else if previous_config_absent.exists() && paths.config_path.exists() {
        fs::remove_file(&paths.config_path).with_context(|| {
            format!(
                "Unable to remove restored config {}",
                paths.config_path.display()
            )
        })?;
    }

    if previous_managed_path.exists() {
        ensure_parent(&paths.managed_netplan_path)?;
        fs::copy(&previous_managed_path, &paths.managed_netplan_path).with_context(|| {
            format!(
                "Unable to restore {} from backup {}",
                paths.managed_netplan_path.display(),
                previous_managed_path.display()
            )
        })?;
    } else if absent_marker.exists() && paths.managed_netplan_path.exists() {
        fs::remove_file(&paths.managed_netplan_path).with_context(|| {
            format!(
                "Unable to remove managed netplan file {}",
                paths.managed_netplan_path.display()
            )
        })?;
    }

    let adoption_source_path_file = backup_root.join("adoption-source.path");
    let adoption_source_backup = backup_root.join("adoption-source.before");
    if adoption_source_path_file.exists() && adoption_source_backup.exists() {
        let source_path = fs::read_to_string(&adoption_source_path_file).with_context(|| {
            format!(
                "Unable to read adoption source metadata {}",
                adoption_source_path_file.display()
            )
        })?;
        let source_path = PathBuf::from(source_path.trim());
        ensure_parent(&source_path)?;
        fs::copy(&adoption_source_backup, &source_path).with_context(|| {
            format!(
                "Unable to restore adoption source {} from backup {}",
                source_path.display(),
                adoption_source_backup.display()
            )
        })?;
    }

    Ok(())
}

fn output_suffix(stdout: &str, stderr: &str) -> String {
    let mut parts = Vec::new();
    if !stdout.is_empty() {
        parts.push(format!("stdout:\n{stdout}"));
    }
    if !stderr.is_empty() {
        parts.push(format!("stderr:\n{stderr}"));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("\n{}", parts.join("\n"))
    }
}

fn describe_status(status: std::process::ExitStatus) -> String {
    if let Some(code) = status.code() {
        format!("exit code {code}")
    } else if let Some(signal) = status.signal() {
        format!("signal {signal}")
    } else {
        "unknown status".to_string()
    }
}

fn log_command_output(prefix: &str, stdout: &str, stderr: &str) {
    if !stdout.is_empty() {
        info!("{prefix} stdout:\n{stdout}");
    }
    if !stderr.is_empty() {
        warn!("{prefix} stderr:\n{stderr}");
    }
}

fn run_netplan_apply(paths: &HelperPaths) -> Result<()> {
    let output = Command::new(&paths.netplan_bin)
        .arg("apply")
        .output()
        .with_context(|| format!("Unable to start {}", paths.netplan_bin.display()))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if output.status.success() {
        info!("netplan apply succeeded");
        log_command_output("netplan apply", &stdout, &stderr);
        Ok(())
    } else {
        warn!(
            "netplan apply failed with {}",
            describe_status(output.status)
        );
        log_command_output("netplan apply", &stdout, &stderr);
        bail!(
            "netplan apply failed with {}{}",
            describe_status(output.status),
            output_suffix(&stdout, &stderr)
        )
    }
}

fn cleanup_pending_record(paths: &HelperPaths, operation_id: &str) {
    let _ = fs::remove_file(pending_record_path(paths, operation_id));
}

fn write_pending_record(paths: &HelperPaths, record: &PendingOperationRecord) -> Result<()> {
    let json =
        serde_json::to_string_pretty(record).context("Unable to serialize pending record")?;
    write_text_file(&pending_record_path(paths, &record.operation_id), &json)
}

fn list_pending_records(paths: &HelperPaths) -> Result<Vec<PendingOperationRecord>> {
    let mut records = Vec::new();
    let Ok(entries) = fs::read_dir(&paths.pending_dir) else {
        return Ok(records);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("Unable to read pending operation {}", path.display()))?;
        let record = serde_json::from_str(&raw).context("Unable to parse pending operation")?;
        records.push(record);
    }
    records.sort_by_key(|record| std::cmp::Reverse(record.created_unix));
    Ok(records)
}

fn pending_status(record: &PendingOperationRecord, state: &str) -> PendingOperationStatus {
    PendingOperationStatus {
        operation_id: record.operation_id.clone(),
        backup_id: record.backup_id.clone(),
        state: state.to_string(),
        source: record.source.clone(),
        operator_username: record.operator_username.clone(),
        summary: record.summary.clone(),
        created_unix: record.created_unix,
    }
}

fn pending_deadline_unix(paths: &HelperPaths, record: &PendingOperationRecord) -> u64 {
    record
        .created_unix
        .saturating_add(u64::from(paths.netplan_timeout_secs))
}

fn schedule_rollback_worker(
    paths: HelperPaths,
    operation_id: String,
    backup_id: String,
    deadline_unix: u64,
) {
    std::thread::spawn(move || {
        let now = unix_now().unwrap_or(0);
        if deadline_unix > now {
            std::thread::sleep(Duration::from_secs(deadline_unix - now));
        }

        let Ok(record) = read_pending_record(&paths, &operation_id) else {
            return;
        };
        if record.backup_id != backup_id {
            return;
        }

        warn!(
            "Pending network change {} reached its rollback deadline; restoring backup {}",
            operation_id, backup_id
        );
        if let Err(err) = restore_backup_files(&paths, &backup_id) {
            warn!(
                "Unable to restore backup {} for expired pending operation {}: {err}",
                backup_id, operation_id
            );
            return;
        }
        if let Err(err) = run_netplan_apply(&paths) {
            warn!(
                "Expired pending operation {} restored backup {}, but netplan apply failed: {err}",
                operation_id, backup_id
            );
            return;
        }
        if let Err(err) = retry_shaping(&paths) {
            warn!(
                "Expired pending operation {} restored backup {}, but shaping retry failed: {err}",
                operation_id, backup_id
            );
        }
        cleanup_pending_record(&paths, &operation_id);
    });
}

fn schedule_pending_rollbacks(
    paths: &HelperPaths,
    pending_children: &mut PendingChildren,
) -> Result<()> {
    let records = list_pending_records(paths)?;
    let live_ids = records
        .iter()
        .map(|record| record.operation_id.clone())
        .collect::<BTreeSet<_>>();
    pending_children
        .scheduled_rollbacks
        .retain(|operation_id| live_ids.contains(operation_id));

    let now = now_unix()?;
    for record in records {
        if now >= pending_deadline_unix(paths, &record)
            || pending_children
                .scheduled_rollbacks
                .contains(&record.operation_id)
        {
            continue;
        }
        schedule_rollback_worker(
            paths.clone(),
            record.operation_id.clone(),
            record.backup_id.clone(),
            pending_deadline_unix(paths, &record),
        );
        pending_children
            .scheduled_rollbacks
            .insert(record.operation_id.clone());
    }
    Ok(())
}

fn normalize_pending_children(
    paths: &HelperPaths,
    pending_children: &mut PendingChildren,
) -> Result<()> {
    let now = now_unix()?;
    for record in list_pending_records(paths)? {
        if now < pending_deadline_unix(paths, &record) {
            continue;
        }
        warn!(
            "Pending network change {} expired; restoring backup {}",
            record.operation_id, record.backup_id
        );
        restore_backup_files(paths, &record.backup_id)?;
        if let Err(err) = run_netplan_apply(paths) {
            bail!(
                "Pending network change {} expired and backup {} was restored, but netplan apply failed: {err}",
                record.operation_id,
                record.backup_id
            );
        }
        cleanup_pending_record(paths, &record.operation_id);
    }
    schedule_pending_rollbacks(paths, pending_children)?;
    Ok(())
}

fn retry_shaping(paths: &HelperPaths) -> Result<()> {
    if let RetryShapingAction::LoadLibreQoS = paths.retry_shaping {
        lqos_config::load_libreqos()
            .map(|_| ())
            .map_err(|err| anyhow!("Unable to retry LibreQoS shaping: {err}"))?;
    }
    Ok(())
}

/// Return current pending-operation and rollback-bundle status for the helper.
pub fn helper_status(
    paths: &HelperPaths,
    pending_children: &mut PendingChildren,
) -> Result<HelperStatus> {
    normalize_pending_children(paths, pending_children)?;
    let pending_operation = list_pending_records(paths)?
        .into_iter()
        .next()
        .map(|record| pending_status(&record, "PendingTry"));

    let recent_backups = list_recent_backups(paths)?;
    Ok(HelperStatus {
        pending_operation,
        last_backup_id: recent_backups
            .first()
            .map(|backup| backup.backup_id.clone()),
        recent_backup_ids: recent_backups
            .iter()
            .map(|backup| backup.backup_id.clone())
            .collect(),
        recent_backups,
    })
}

fn dangerous_change_error(inspection: &NetworkModeInspection) -> anyhow::Error {
    let mut message =
        String::from("Strong confirmation is required before applying this network change:");
    for warning in &inspection.dangerous_changes {
        message.push_str("\n- ");
        message.push_str(warning);
    }
    if let Some(text) = &inspection.strong_confirmation_text {
        message.push_str("\n\n");
        message.push_str(text);
    }
    anyhow!(message)
}

fn validate_apply_request(
    request: &ApplyRequest,
    inspection: &NetworkModeInspection,
    previous_config: &Config,
) -> Result<()> {
    if !inspection.dangerous_changes.is_empty() && !request.confirm_dangerous_changes {
        return Err(dangerous_change_error(inspection));
    }

    if mode_label(previous_config) != mode_label(&request.config)
        && !request.confirm_dangerous_changes
    {
        bail!(
            "Switching between {} and {} requires strong confirmation.",
            mode_label(previous_config),
            mode_label(&request.config)
        );
    }

    match request.mode {
        ApplyMode::Apply => {
            if inspection.can_take_over {
                bail!(
                    "Take Over is required before LibreQoS can manage the existing libreqos.yaml."
                );
            }
            if inspection.can_adopt {
                bail!(
                    "Adopt into libreqos.yaml is required before LibreQoS can manage the external compatible netplan file."
                );
            }
            if inspection.inspector_state != "Ready"
                && inspection.inspector_state != "ManagedByLibreQoS"
            {
                bail!("{}", inspection.summary);
            }
        }
        ApplyMode::Adopt => {
            if !inspection.can_adopt {
                bail!("Adoption is not available for the current netplan state.");
            }
        }
        ApplyMode::TakeOver => {
            if !inspection.can_take_over {
                bail!("Take Over is not available for the current netplan state.");
            }
        }
    }

    Ok(())
}

/// Stage and apply a managed LibreQoS network-mode transaction.
pub fn apply_transaction(
    paths: &HelperPaths,
    pending_children: &mut PendingChildren,
    request: ApplyRequest,
) -> Result<ApplyResponse> {
    normalize_pending_children(paths, pending_children)?;
    if !list_pending_records(paths)?.is_empty() {
        bail!("A pending network change already exists. Confirm or revert it first.");
    }

    let previous_config = if paths.config_path.exists() {
        load_config_from_path(&paths.config_path)?
    } else {
        Config::default()
    };
    let inspection = inspect_with_paths(paths, &request.config);
    validate_apply_request(&request, &inspection, &previous_config)?;

    let preview_yaml = inspection.managed_preview_yaml.as_ref().ok_or_else(|| {
        anyhow!(
            "{}",
            inspection.preview_note.clone().unwrap_or_else(|| {
                "Managed netplan preview is not available for this mode.".to_string()
            })
        )
    })?;

    fs::create_dir_all(&paths.backup_dir)
        .with_context(|| format!("Unable to create {}", paths.backup_dir.display()))?;
    fs::create_dir_all(&paths.pending_dir)
        .with_context(|| format!("Unable to create {}", paths.pending_dir.display()))?;

    let external_source_path = if request.mode == ApplyMode::Adopt {
        Some(PathBuf::from(
            inspection.adopt_source_path.clone().ok_or_else(|| {
                anyhow!("No external netplan source was identified for adoption.")
            })?,
        ))
    } else {
        None
    };

    let operation_id = Uuid::new_v4().to_string();
    let backup_id = write_backup_bundle(
        paths,
        &previous_config,
        &request,
        &inspection,
        &operation_id,
        external_source_path.as_deref(),
    )?;

    let write_result = (|| -> Result<()> {
        write_config_to_path(&paths.config_path, &request.config)?;
        if let Some(source_path) = external_source_path.as_ref() {
            let rewritten = adoption_rewrite_for_path(source_path, &request.config)
                .map_err(|err| anyhow!("{err}"))?;
            write_netplan_file(source_path, &rewritten)?;
        }
        write_netplan_file(&paths.managed_netplan_path, preview_yaml)?;
        Ok(())
    })();
    if let Err(err) = write_result {
        restore_backup_files(paths, &backup_id)?;
        let _ = run_netplan_apply(paths);
        return Err(err);
    }
    if let Err(err) = run_netplan_apply(paths) {
        restore_backup_files(paths, &backup_id)?;
        let rollback_result = run_netplan_apply(paths);
        return Err(match rollback_result {
            Ok(()) => anyhow!("Network changes were written, but netplan apply failed: {err}"),
            Err(rollback_err) => anyhow!(
                "Network changes were written, netplan apply failed, and rollback apply also failed: {err}; rollback error: {rollback_err}"
            ),
        });
    }

    let record = PendingOperationRecord {
        operation_id: operation_id.clone(),
        backup_id: backup_id.clone(),
        source: request.source.clone(),
        operator_username: request.operator_username.clone(),
        created_unix: now_unix()?,
        summary: match request.mode {
            ApplyMode::Apply => "Network changes were applied. Confirm within 30 seconds or LibreQoS will roll back.".to_string(),
            ApplyMode::Adopt => "Adopted the compatible external netplan config into libreqos.yaml. Confirm within 30 seconds or LibreQoS will roll back.".to_string(),
            ApplyMode::TakeOver => "Took over the existing libreqos.yaml. Confirm within 30 seconds or LibreQoS will roll back.".to_string(),
        },
        pid: None,
    };
    if let Err(err) = write_pending_record(paths, &record) {
        restore_backup_files(paths, &backup_id)?;
        let rollback_result = run_netplan_apply(paths);
        return Err(match rollback_result {
            Ok(()) => anyhow!(
                "Network changes were applied, but LibreQoS could not persist the pending confirmation state: {err}"
            ),
            Err(rollback_err) => anyhow!(
                "Network changes were applied, LibreQoS could not persist the pending confirmation state, and rollback apply failed: {err}; rollback error: {rollback_err}"
            ),
        });
    }
    schedule_pending_rollbacks(paths, pending_children)?;

    Ok(ApplyResponse {
        ok: true,
        message: "Network changes applied. Confirm within 30 seconds or LibreQoS will roll back."
            .to_string(),
        operation: Some(pending_status(&record, "PendingTry")),
        last_backup_id: Some(backup_id),
    })
}

/// Confirm a pending network change and optionally retry shaping.
pub fn confirm_transaction(
    paths: &HelperPaths,
    pending_children: &mut PendingChildren,
    operation_id: &str,
) -> Result<ApplyResponse> {
    normalize_pending_children(paths, pending_children)?;
    let record = read_pending_record(paths, operation_id)?;
    cleanup_pending_record(paths, operation_id);
    pending_children.scheduled_rollbacks.remove(operation_id);

    retry_shaping(paths)?;

    Ok(ApplyResponse {
        ok: true,
        message: "Network changes confirmed.".to_string(),
        operation: Some(pending_status(&record, "Confirmed")),
        last_backup_id: Some(record.backup_id),
    })
}

/// Explicitly revert a pending network change.
pub fn revert_transaction(
    paths: &HelperPaths,
    pending_children: &mut PendingChildren,
    operation_id: &str,
) -> Result<ApplyResponse> {
    normalize_pending_children(paths, pending_children)?;
    let record = read_pending_record(paths, operation_id)?;
    restore_backup_files(paths, &record.backup_id)?;
    if let Err(err) = run_netplan_apply(paths) {
        bail!("Revert restored files but netplan apply failed: {err}");
    }
    cleanup_pending_record(paths, operation_id);
    pending_children.scheduled_rollbacks.remove(operation_id);
    retry_shaping(paths)?;

    Ok(ApplyResponse {
        ok: true,
        message: "Network changes reverted and previous managed files restored.".to_string(),
        operation: Some(pending_status(&record, "Reverted")),
        last_backup_id: Some(record.backup_id),
    })
}

/// Restore a previously recorded rollback bundle.
pub fn rollback_transaction(
    paths: &HelperPaths,
    pending_children: &mut PendingChildren,
    backup_id: &str,
) -> Result<ApplyResponse> {
    normalize_pending_children(paths, pending_children)?;
    if !list_pending_records(paths)?.is_empty() {
        bail!("A pending network change already exists. Confirm or revert it first.");
    }

    read_backup_manifest(paths, backup_id)?;
    restore_backup_files(paths, backup_id)?;

    if let Err(err) = run_netplan_apply(paths) {
        bail!("Rollback restored files but netplan apply failed: {err}");
    }

    retry_shaping(paths)?;

    Ok(ApplyResponse {
        ok: true,
        message: format!("Rollback bundle {backup_id} restored."),
        operation: None,
        last_backup_id: Some(backup_id.to_string()),
    })
}

/// Trigger a LibreQoS shaping retry without changing the active netplan state.
pub fn retry_shaping_transaction(
    paths: &HelperPaths,
    pending_children: &mut PendingChildren,
) -> Result<ApplyResponse> {
    normalize_pending_children(paths, pending_children)?;
    retry_shaping(paths)?;
    Ok(ApplyResponse {
        ok: true,
        message: "LibreQoS shaping retry requested.".to_string(),
        operation: None,
        last_backup_id: helper_status(paths, pending_children)?.last_backup_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn helper_paths(root: &Path) -> HelperPaths {
        HelperPaths {
            config_path: root.join("etc/lqos.conf"),
            netplan_dir: root.join("etc/netplan"),
            managed_netplan_path: root.join("etc/netplan/libreqos.yaml"),
            backup_dir: root.join("var/lib/libreqos/netplan-backups"),
            pending_dir: root.join("var/lib/libreqos/netplan-pending"),
            netplan_bin: root.join("bin/netplan"),
            netplan_timeout_secs: 30,
            retry_shaping: RetryShapingAction::None,
        }
    }

    fn base_dir(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("lqos-netplan-tx-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("etc/netplan")).expect("create netplan dir");
        fs::create_dir_all(root.join("bin")).expect("create bin dir");
        root
    }

    fn write_netplan_stub(root: &Path) {
        let script = root.join("bin/netplan");
        let log_path = root.join("netplan.log");
        let body = r#"#!/bin/bash
set -euo pipefail
echo "$*" >> "__LOG_PATH__"
cmd="$1"
shift || true
case "$cmd" in
  apply)
    exit 0
    ;;
  *)
    exit 1
    ;;
esac
"#;
        fs::write(
            &script,
            body.replace("__LOG_PATH__", &log_path.display().to_string()),
        )
        .expect("write stub netplan");
        let mut perms = fs::metadata(&script).expect("stat stub").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).expect("chmod stub");
    }

    fn write_config(path: &Path, config: &Config) {
        write_config_to_path(path, config).expect("write config");
    }

    fn test_interfaces() -> (String, String) {
        let interfaces = system_interfaces();
        let primary = interfaces
            .iter()
            .find(|iface| iface.as_str() != "lo")
            .cloned()
            .or_else(|| interfaces.iter().next().cloned())
            .expect("test host should expose at least one interface");
        let secondary = interfaces
            .iter()
            .find(|iface| iface.as_str() != primary && iface.as_str() != "lo")
            .cloned()
            .or_else(|| {
                interfaces
                    .iter()
                    .find(|iface| iface.as_str() != primary)
                    .cloned()
            })
            .expect("test host should expose a second distinct interface");
        (primary, secondary)
    }

    fn linux_bridge_config() -> Config {
        let (to_internet, to_network) = test_interfaces();
        Config {
            bridge: Some(lqos_config::BridgeConfig {
                use_xdp_bridge: false,
                to_internet,
                to_network,
            }),
            single_interface: None,
            ..Config::default()
        }
    }

    #[test]
    fn apply_confirm_and_revert_manage_pending_records() {
        let root = base_dir("apply-confirm");
        write_netplan_stub(&root);
        let paths = helper_paths(&root);
        let mut pending = PendingChildren::default();
        let existing = linux_bridge_config();
        write_config(&paths.config_path, &existing);

        let request = ApplyRequest {
            config: existing.clone(),
            source: "ui".to_string(),
            operator_username: Some("admin".to_string()),
            mode: ApplyMode::Apply,
            confirm_dangerous_changes: true,
        };

        let apply = apply_transaction(&paths, &mut pending, request).expect("apply should succeed");
        assert!(apply.ok);
        assert!(apply.operation.is_some());

        let operation_id = apply
            .operation
            .as_ref()
            .expect("pending op")
            .operation_id
            .clone();
        assert!(pending_record_path(&paths, &operation_id).exists());

        let confirm =
            confirm_transaction(&paths, &mut pending, &operation_id).expect("confirm should work");
        assert!(confirm.ok);
        assert!(!pending_record_path(&paths, &operation_id).exists());

        let request = ApplyRequest {
            config: existing,
            source: "ui".to_string(),
            operator_username: Some("admin".to_string()),
            mode: ApplyMode::Apply,
            confirm_dangerous_changes: true,
        };
        let apply = apply_transaction(&paths, &mut pending, request).expect("second apply");
        let operation_id = apply
            .operation
            .as_ref()
            .expect("pending op")
            .operation_id
            .clone();
        let revert =
            revert_transaction(&paths, &mut pending, &operation_id).expect("revert should work");
        assert!(revert.ok);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rollback_restores_previous_files() {
        let root = base_dir("rollback");
        write_netplan_stub(&root);
        let paths = helper_paths(&root);
        let mut pending = PendingChildren::default();
        let existing = linux_bridge_config();
        write_config(&paths.config_path, &existing);
        write_text_file(&paths.managed_netplan_path, "network:\n  version: 2\n")
            .expect("seed managed file");

        let mut next = existing.clone();
        next.single_interface = Some(lqos_config::SingleInterfaceConfig {
            interface: test_interfaces().0,
            internet_vlan: 2,
            network_vlan: 3,
        });
        next.bridge = None;

        let apply = apply_transaction(
            &paths,
            &mut pending,
            ApplyRequest {
                config: next,
                source: "ui".to_string(),
                operator_username: Some("admin".to_string()),
                mode: ApplyMode::Apply,
                confirm_dangerous_changes: true,
            },
        )
        .expect("apply should succeed");
        let backup_id = apply
            .last_backup_id
            .clone()
            .expect("backup id should be recorded");
        let operation_id = apply
            .operation
            .as_ref()
            .expect("pending op")
            .operation_id
            .clone();

        revert_transaction(&paths, &mut pending, &operation_id).expect("revert should work");

        write_config(
            &paths.config_path,
            &Config {
                single_interface: Some(lqos_config::SingleInterfaceConfig {
                    interface: test_interfaces().1,
                    internet_vlan: 10,
                    network_vlan: 11,
                }),
                bridge: None,
                ..Config::default()
            },
        );
        write_text_file(&paths.managed_netplan_path, "broken: true\n").expect("mutate managed");

        let rollback =
            rollback_transaction(&paths, &mut pending, &backup_id).expect("rollback should work");
        assert!(rollback.ok);

        let restored_config =
            load_config_from_path(&paths.config_path).expect("load restored config");
        assert_eq!(restored_config.bridge, existing.bridge);
        assert_eq!(
            fs::read_to_string(&paths.managed_netplan_path).expect("read managed"),
            "network:\n  version: 2\n"
        );
        let log = fs::read_to_string(root.join("netplan.log")).expect("read netplan log");
        assert!(log.lines().any(|line| line == "apply"));
        assert!(!log.lines().any(|line| line == "generate"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn revert_applies_restored_netplan() {
        let root = base_dir("revert-apply");
        write_netplan_stub(&root);
        let paths = helper_paths(&root);
        let mut pending = PendingChildren::default();
        let existing = linux_bridge_config();
        write_config(&paths.config_path, &existing);

        let apply = apply_transaction(
            &paths,
            &mut pending,
            ApplyRequest {
                config: existing,
                source: "ui".to_string(),
                operator_username: Some("admin".to_string()),
                mode: ApplyMode::Apply,
                confirm_dangerous_changes: true,
            },
        )
        .expect("apply should succeed");
        let operation_id = apply
            .operation
            .as_ref()
            .expect("pending op")
            .operation_id
            .clone();

        let revert =
            revert_transaction(&paths, &mut pending, &operation_id).expect("revert should work");
        assert!(revert.ok);

        let log = fs::read_to_string(root.join("netplan.log")).expect("read netplan log");
        assert!(log.lines().any(|line| line == "apply"));
        assert!(!log.lines().any(|line| line == "generate"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn expired_pending_operation_rolls_back_on_status_check() {
        let root = base_dir("expire-rollback");
        write_netplan_stub(&root);
        let mut paths = helper_paths(&root);
        paths.netplan_timeout_secs = 0;
        let mut pending = PendingChildren::default();
        let existing = linux_bridge_config();
        write_config(&paths.config_path, &existing);
        write_text_file(&paths.managed_netplan_path, "network:\n  version: 2\n")
            .expect("seed managed file");

        let mut next = existing.clone();
        next.single_interface = Some(lqos_config::SingleInterfaceConfig {
            interface: test_interfaces().0,
            internet_vlan: 2,
            network_vlan: 3,
        });
        next.bridge = None;

        let apply = apply_transaction(
            &paths,
            &mut pending,
            ApplyRequest {
                config: next,
                source: "ui".to_string(),
                operator_username: Some("admin".to_string()),
                mode: ApplyMode::Apply,
                confirm_dangerous_changes: true,
            },
        )
        .expect("apply should succeed");
        let operation_id = apply
            .operation
            .as_ref()
            .expect("pending op")
            .operation_id
            .clone();

        let status = helper_status(&paths, &mut pending).expect("status should expire pending");
        assert!(status.pending_operation.is_none());
        assert!(!pending_record_path(&paths, &operation_id).exists());

        let restored_config =
            load_config_from_path(&paths.config_path).expect("load restored config");
        assert_eq!(restored_config.bridge, existing.bridge);
        assert_eq!(
            fs::read_to_string(&paths.managed_netplan_path).expect("read managed"),
            "network:\n  version: 2\n"
        );
        let log = fs::read_to_string(root.join("netplan.log")).expect("read netplan log");
        assert_eq!(log.lines().filter(|line| *line == "apply").count(), 2);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn apply_reports_netplan_apply_stderr_on_failure() {
        let root = base_dir("apply-failure");
        let script = root.join("bin/netplan");
        let body = r#"#!/bin/bash
set -euo pipefail
cmd="$1"
shift || true
case "$cmd" in
  apply)
    echo "systemd-networkd could not read generated files" >&2
    exit 1
    ;;
  *)
    exit 1
    ;;
esac
"#;
        fs::write(&script, body).expect("write failing stub");
        let mut perms = fs::metadata(&script).expect("stat stub").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).expect("chmod stub");

        let paths = helper_paths(&root);
        let mut pending = PendingChildren::default();
        let existing = linux_bridge_config();
        write_config(&paths.config_path, &existing);

        let err = apply_transaction(
            &paths,
            &mut pending,
            ApplyRequest {
                config: existing,
                source: "ui".to_string(),
                operator_username: Some("admin".to_string()),
                mode: ApplyMode::Apply,
                confirm_dangerous_changes: true,
            },
        )
        .expect_err("apply should fail");

        let message = err.to_string();
        assert!(message.contains("netplan apply failed"));
        assert!(message.contains("systemd-networkd could not read generated files"));

        let _ = fs::remove_dir_all(root);
    }
}
