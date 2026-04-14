//! Runtime service handoff helpers for completing first-run setup.

use anyhow::{Context, Result, bail};
use std::process::Command;
use uuid::Uuid;

const SYSTEMD_RUN_BIN: &str = "/usr/bin/systemd-run";
const SYSTEMCTL_BIN: &str = "/bin/systemctl";

pub(crate) struct HandoffNotice {
    pub(crate) message: String,
    pub(crate) automatic: bool,
}

fn ensure_root_with_systemctl() -> Result<()> {
    if unsafe { libc::geteuid() } != 0 {
        bail!("This operation requires root privileges.");
    }
    if !std::path::Path::new(SYSTEMCTL_BIN).exists() {
        bail!("{SYSTEMCTL_BIN} is not available on this host.");
    }
    Ok(())
}

fn run_systemctl(args: &[&str], required: bool) -> Result<()> {
    let status = Command::new(SYSTEMCTL_BIN)
        .args(args)
        .status()
        .with_context(|| format!("Unable to invoke {SYSTEMCTL_BIN} {}", args.join(" ")))?;
    if required && !status.success() {
        bail!(
            "{SYSTEMCTL_BIN} {} failed with exit status {status}",
            args.join(" ")
        );
    }
    Ok(())
}

fn stop_and_disable(services: &[&str]) -> Result<()> {
    if services.is_empty() {
        return Ok(());
    }

    let mut stop_args = vec!["stop"];
    stop_args.extend_from_slice(services);
    run_systemctl(&stop_args, false)?;

    let mut disable_args = vec!["disable"];
    disable_args.extend_from_slice(services);
    run_systemctl(&disable_args, false)
}

fn enable_and_restart(services: &[&str]) -> Result<()> {
    if services.is_empty() {
        return Ok(());
    }

    let mut enable_args = vec!["enable"];
    enable_args.extend_from_slice(services);
    run_systemctl(&enable_args, true)?;

    let mut restart_args = vec!["restart"];
    restart_args.extend_from_slice(services);
    run_systemctl(&restart_args, true)
}

fn switch_service_mode(
    stop_disable: &[&str],
    enable_restart: &[&str],
    success_message: &str,
) -> Result<String> {
    ensure_root_with_systemctl()?;
    stop_and_disable(stop_disable)?;

    if let Err(err) = enable_and_restart(enable_restart) {
        let _ = stop_and_disable(enable_restart);
        let rollback = enable_and_restart(stop_disable);
        match rollback {
            Ok(()) => {
                bail!(
                    "Unable to activate the target service mode: {err:#}. Restored the previous service mode."
                )
            }
            Err(rollback_err) => {
                bail!(
                    "Unable to activate the target service mode: {err:#}. Rollback to the previous service mode also failed: {rollback_err:#}"
                )
            }
        }
    }

    Ok(success_message.to_string())
}

/// Stops setup and starts the runtime services immediately.
///
/// Side effects: disables and stops `lqos_setup.service`, then enables and
/// restarts `lqosd.service` and `lqos_scheduler.service`.
pub(crate) fn activate_runtime_services() -> Result<String> {
    switch_service_mode(
        &["lqos_setup.service"],
        &[
            "lqosd.service",
            "lqos_scheduler.service",
            "lqos_api.service",
        ],
        "Activated runtime services: lqosd, lqos_scheduler, and lqos_api.",
    )
}

/// Stops runtime services and starts the setup service immediately.
///
/// Side effects: stops and disables `lqosd.service` and
/// `lqos_scheduler.service`, then enables and restarts `lqos_setup.service`.
pub(crate) fn activate_setup_service() -> Result<String> {
    switch_service_mode(
        &[
            "lqosd.service",
            "lqos_scheduler.service",
            "lqos_api.service",
        ],
        &["lqos_setup.service"],
        "Activated first-run setup service: lqos_setup.",
    )
}

/// Schedules a transient systemd unit that stops setup and starts runtime services.
///
/// Side effects: spawns a transient systemd service, disables and stops
/// `lqos_setup.service`, and enables and restarts `lqosd.service` and
/// `lqos_scheduler.service` after a short delay.
pub(crate) fn schedule_runtime_handoff() -> Result<HandoffNotice> {
    if std::env::var_os("INVOCATION_ID").is_none() {
        return Ok(HandoffNotice {
            message: "Setup saved. Start lqosd, lqos_scheduler, and lqos_api manually because this setup run is not being managed by systemd.".to_string(),
            automatic: false,
        });
    }

    if unsafe { libc::geteuid() } != 0 {
        return Ok(HandoffNotice {
            message: "Setup saved. Start lqosd, lqos_scheduler, and lqos_api manually because the current setup process is not running as root.".to_string(),
            automatic: false,
        });
    }

    if !std::path::Path::new(SYSTEMD_RUN_BIN).exists() {
        return Ok(HandoffNotice {
            message: "Setup saved. Start lqosd, lqos_scheduler, and lqos_api manually because systemd-run is not available on this host.".to_string(),
            automatic: false,
        });
    }

    ensure_root_with_systemctl()?;
    let unit_name = format!("lqos-setup-handoff-{}", Uuid::new_v4().simple());
    let current_exe = std::env::current_exe().context("Unable to determine lqos_setup path")?;
    let status = Command::new(SYSTEMD_RUN_BIN)
        .args([
            "--unit",
            &unit_name,
            "--on-active=2s",
            current_exe.to_string_lossy().as_ref(),
            "activate-runtime",
        ])
        .status()
        .with_context(|| format!("Unable to invoke {SYSTEMD_RUN_BIN} for setup handoff"))?;

    if !status.success() {
        bail!(
            "Unable to schedule runtime handoff via systemd-run (exit status: {status}). Start lqosd and lqos_scheduler manually."
        );
    }

    Ok(HandoffNotice {
        message: format!(
            "Scheduled runtime handoff via transient unit {unit_name}. LibreQoS setup will stop and lqosd/lqos_scheduler/lqos_api will start shortly."
        ),
        automatic: true,
    })
}
