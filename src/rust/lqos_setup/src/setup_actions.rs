//! Shared setup commit/apply helpers used by both Cursive and the setup WebUI.

use crate::config_builder::{BridgeMode, CURRENT_CONFIG};
use anyhow::{Context, Result, bail};
use lqos_netplan_helper::protocol::{ApplyMode, ApplyRequest};
use lqos_netplan_helper::transaction::{
    HelperPaths, PendingChildren, apply_transaction, confirm_transaction, inspect_with_paths,
    revert_transaction,
};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

static PENDING_CHILDREN: Lazy<Mutex<PendingChildren>> =
    Lazy::new(|| Mutex::new(PendingChildren::default()));
static PENDING_COMMITS: Lazy<Mutex<HashMap<String, PendingCommitState>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

struct PendingCommitState {
    config: lqos_config::Config,
    event_log: Vec<String>,
}

/// Successful setup commit result.
pub(crate) struct CommitSuccess {
    pub(crate) config: lqos_config::Config,
    pub(crate) event_log: Vec<String>,
}

/// Pending network confirmation data.
pub(crate) struct PendingCommit {
    pub(crate) operation_id: String,
    pub(crate) prompt: String,
}

/// Outcome of attempting to commit setup settings.
pub(crate) enum CommitOutcome {
    Complete(Box<CommitSuccess>),
    Pending(PendingCommit),
}

pub(crate) fn build_candidate_config(existing: Option<lqos_config::Config>) -> lqos_config::Config {
    let mut config = existing.unwrap_or_default();
    let new_config = CURRENT_CONFIG.lock();
    config.node_name = new_config.node_name.clone();
    config.queues.downlink_bandwidth_mbps = new_config.mbps_to_internet;
    config.queues.uplink_bandwidth_mbps = new_config.mbps_to_network;
    config.queues.generated_pn_download_mbps = new_config.mbps_to_internet;
    config.queues.generated_pn_upload_mbps = new_config.mbps_to_network;
    match new_config.bridge_mode {
        BridgeMode::Linux => {
            config.single_interface = None;
            config.bridge = Some(lqos_config::BridgeConfig {
                use_xdp_bridge: false,
                to_internet: new_config.to_internet.clone(),
                to_network: new_config.to_network.clone(),
            });
        }
        BridgeMode::XDP => {
            config.single_interface = None;
            config.bridge = Some(lqos_config::BridgeConfig {
                use_xdp_bridge: true,
                to_internet: new_config.to_internet.clone(),
                to_network: new_config.to_network.clone(),
            });
        }
        BridgeMode::Single => {
            config.single_interface = Some(lqos_config::SingleInterfaceConfig {
                interface: new_config.to_internet.clone(),
                internet_vlan: new_config.internet_vlan,
                network_vlan: new_config.network_vlan,
            });
            config.bridge = None;
        }
    }
    config.ip_ranges.allow_subnets = new_config.allow_subnets.clone();
    config
}

pub(crate) fn inspection_report(inspection: &lqos_netplan_helper::NetworkModeInspection) -> String {
    let mut report = format!(
        "State: {}\n{}\n",
        inspection.inspector_state, inspection.summary
    );
    if !inspection.warnings.is_empty() {
        report.push_str("\nWarnings:\n");
        for warning in &inspection.warnings {
            let _ = writeln!(&mut report, "- {warning}");
        }
    }
    if !inspection.dangerous_changes.is_empty() {
        report.push_str("\nStrong confirmations required:\n");
        for warning in &inspection.dangerous_changes {
            let _ = writeln!(&mut report, "- {warning}");
        }
    }
    if !inspection.conflicts.is_empty() {
        report.push_str("\nConflicts:\n");
        for conflict in &inspection.conflicts {
            let _ = writeln!(&mut report, "- {conflict}");
        }
    }
    if let Some(preview) = &inspection.managed_preview_yaml {
        report.push_str("\nManaged Preview:\n");
        report.push_str(preview);
    } else if let Some(note) = &inspection.preview_note {
        report.push('\n');
        report.push_str(note);
    }
    report
}

pub(crate) fn prepare_commit() -> Result<CommitOutcome> {
    if !lqos_setup::bootstrap::first_admin_exists() {
        bail!("Setup requires at least one admin user before configuration can be committed.");
    }

    let mut event_log = Vec::new();
    let existing_config = load_existing_or_default(&mut event_log)?;
    let config = build_candidate_config(Some(existing_config));
    let using_helper = !matches!(CURRENT_CONFIG.lock().bridge_mode, BridgeMode::XDP);

    if !using_helper {
        lqos_config::update_config(&config)?;
        event_log.push("Configuration updated.".to_string());
        return Ok(CommitOutcome::Complete(Box::new(CommitSuccess {
            config,
            event_log,
        })));
    }

    let helper_paths = HelperPaths::default();
    let inspection = inspect_with_paths(&helper_paths, &config);
    let bootstrap_incomplete = lqos_setup::bootstrap::setup_is_incomplete().unwrap_or(true);
    let apply_mode = if inspection.can_take_over {
        ApplyMode::TakeOver
    } else if inspection.can_adopt {
        ApplyMode::Adopt
    } else {
        ApplyMode::Apply
    };
    let response = {
        let mut pending = PENDING_CHILDREN.lock();
        apply_transaction(
            &helper_paths,
            &mut pending,
            ApplyRequest {
                config: config.clone(),
                source: "setup".to_string(),
                operator_username: None,
                mode: apply_mode,
                confirm_dangerous_changes: bootstrap_incomplete
                    || !inspection.dangerous_changes.is_empty(),
            },
        )
    }?;

    let Some(operation) = response.operation else {
        return Ok(CommitOutcome::Complete(Box::new(CommitSuccess {
            config,
            event_log,
        })));
    };

    PENDING_COMMITS.lock().insert(
        operation.operation_id.clone(),
        PendingCommitState { config, event_log },
    );

    Ok(CommitOutcome::Pending(PendingCommit {
        operation_id: operation.operation_id,
        prompt: format!(
            "{}\n\n{}\n\nConfirm the change to keep the managed netplan update, or revert it now.",
            response.message,
            inspection_report(&inspection)
        ),
    }))
}

pub(crate) fn confirm_pending_commit(operation_id: &str) -> Result<CommitSuccess> {
    let confirm = {
        let mut pending = PENDING_CHILDREN.lock();
        confirm_transaction(&HelperPaths::default(), &mut pending, operation_id)
    }?;

    let Some(mut pending_state) = PENDING_COMMITS.lock().remove(operation_id) else {
        bail!("Pending setup confirmation state for {operation_id} was not found.");
    };
    pending_state.event_log.push(confirm.message);
    Ok(CommitSuccess {
        config: pending_state.config,
        event_log: pending_state.event_log,
    })
}

pub(crate) fn revert_pending_commit(operation_id: &str) -> Result<String> {
    let revert = {
        let mut pending = PENDING_CHILDREN.lock();
        revert_transaction(&HelperPaths::default(), &mut pending, operation_id)
    }?;
    PENDING_COMMITS.lock().remove(operation_id);
    Ok(revert.message)
}

pub(crate) fn persist_setup_success(
    config: &lqos_config::Config,
    event_log: &mut Vec<String>,
) -> Result<()> {
    let state_root = config.resolved_state_directory();
    for category in [
        "topology",
        "shaping",
        "stats",
        "cache",
        "debug",
        "quarantine",
    ] {
        std::fs::create_dir_all(state_root.join(category)).with_context(|| {
            format!(
                "Unable to create setup runtime state directory {}",
                state_root.join(category).display()
            )
        })?;
    }

    lqos_setup::bootstrap::record_setup_success(config)?;
    event_log.push("Setup state updated: Setup Complete".to_string());
    Ok(())
}

fn load_existing_or_default(event_log: &mut Vec<String>) -> Result<lqos_config::Config> {
    if let Ok(config) = lqos_config::load_config() {
        event_log.push("Loaded existing configuration".to_string());
        return Ok((*config).clone());
    }

    let config_path = Path::new("/etc/lqos.conf");
    if config_path.exists() {
        let backup_path = "/etc/lqos.conf.setupbackup";
        std::fs::copy(config_path, backup_path).with_context(|| {
            format!(
                "Existing /etc/lqos.conf could not be loaded and could not be backed up to {backup_path}"
            )
        })?;
        event_log.push(format!(
            "Existing /etc/lqos.conf could not be loaded. Backup saved to {backup_path}."
        ));
    } else {
        event_log.push("Creating new configuration".to_string());
    }

    Ok(lqos_config::Config::default())
}
