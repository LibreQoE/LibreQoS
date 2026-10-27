//! The installer step for the host CPU power posture
//! (`docs/power_profile_work_order.md` §4/§5).
//!
//! Bare metal: apply the `[power] profile` from lqos.conf live (the shipped
//! `libreqos-power.service` re-applies it at every boot). VM: never write from
//! the guest — print the exact host-side steps instead.

use anyhow::{Context, Result};

use lqos_power::ApplyReport;

/// Apply the configured profile from the canonical lqos.conf.
///
/// `force` overrides the A2 conflict REFUSE: the installer passes `true` (a
/// conflicting daemon must never block an install) and surfaces the conflict
/// loudly; the operator CLI passes `false`.
pub fn apply_live(dry_run: bool, force: bool) -> Result<ApplyReport> {
    let config_path = lqos_config::config_path();
    lqos_power::apply_from_config(
        &config_path,
        lqos_power::apply::ORIGINALS_PATH,
        dry_run,
        force,
    )
    .with_context(|| format!("apply the [power] profile from {config_path}"))
}

/// A human-readable summary of an apply (used by both the installer step and
/// the `power-apply` CLI).
pub fn render(report: &ApplyReport) -> String {
    let mut out = String::new();
    if !report.conflicts.is_empty() {
        out.push_str(&format!(
            "WARNING: {} is active and may revert/fight the power profile; \
             the operator should be told to disable it or accept the conflict.\n",
            report.conflicts.join(", ")
        ));
    }
    if report.virtualized {
        out.push_str(
            "This host is a VM: the guest cannot set the host CPU policy. \
             Run these on the bare-metal HOST:\n",
        );
        for step in &report.host_steps {
            out.push_str(step);
            out.push('\n');
        }
    } else {
        out.push_str(&format!(
            "power profile '{}' applied ({} setting(s), {} changed).\n",
            report.profile,
            report.writes.len(),
            report.changed()
        ));
        if report.forced {
            out.push_str("(forced despite an active conflicting control service)\n");
        }
        if !report.unverified.is_empty() {
            out.push_str(&format!(
                "WARNING: {} setting(s) did NOT stick (a conflicting daemon rewrote them).\n",
                report.unverified.len()
            ));
        }
    }
    out
}

/// Best-effort apply used by the runtime activation path (a power failure must
/// never block the install).
pub fn apply_at_activation() {
    match apply_live(false, true) {
        Ok(report) => eprint!("{}", render(&report)),
        Err(err) => eprintln!("WARNING: the [power] profile was not applied: {err:#}"),
    }
}
