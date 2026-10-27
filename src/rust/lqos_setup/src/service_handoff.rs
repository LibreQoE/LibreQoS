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

/// The activation config load, with NO `Config::default()` fallback: a broken
/// config must surface its named error before the topology step (lqos-74t.37.7
/// pin). Kept separate so the no-default rule is unit-testable without the
/// process-global config cache.
fn activation_config(
    loaded: Result<std::sync::Arc<lqos_config::Config>, String>,
) -> Result<lqos_config::Config> {
    loaded
        .map(|config| (*config).clone())
        .map_err(|error| {
            anyhow::anyhow!(
                "cannot load the configuration before activation ({error}); refusing to \
                 activate without the bridge topology"
            )
        })
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

    // `.37.14` P3: the bridge start can wait (bounded) for the fresh-boot
    // transaction, so it must NOT block the caller (dpkg/activation) in the
    // gate. Queue it and continue; lqosd/scheduler/api keep the synchronous
    // contract. A failed queue is reported but is not fatal (the unit is
    // enabled and the operator/next boot will start it).
    for service in services {
        if *service == "libreqos-bridge.service" {
            run_systemctl(&["restart", "--no-block", service], false)?;
        } else {
            run_systemctl(&["restart", service], true)?;
        }
    }
    Ok(())
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

/// The units runtime activation enables + restarts, in order. The bridge is
/// explicit (lqos-74t.37.11: the bind unit no longer pulls it).
const RUNTIME_UNITS: &[&str] = &[
    "libreqos-lqosd.service",
    "libreqos-bridge.service",
    "libreqos-scheduler.service",
    "libreqos-api.service",
];

/// Stops setup and starts the runtime services immediately.
///
/// Side effects: disables and stops `libreqos-first-run.service`, then enables and
/// restarts `libreqos-lqosd.service`, `libreqos-bridge.service`,
/// `libreqos-scheduler.service`, and `libreqos-api.service` in that order (lqosd
/// first for the v2→v3 migration; the scheduler/analytics attach to a live
/// dataplane).
pub(crate) fn activate_runtime_services() -> Result<String> {
    // The legacy v2 config path is the only one on a v2 box, while the runtime
    // units pin `LQOS_CONFIG` to the canonical v3 path; migrate (copy-only)
    // before the units start or lqosd crash-loops on the missing path.
    if let Some(migration) = crate::bootstrap::migrate_legacy_config()? {
        eprintln!(
            "WARNING: migrated the legacy v2 configuration {} -> {} (the original is untouched); \
             the v3 runtime units read the canonical path.",
            migration.from.display(),
            migration.to.display()
        );
    }
    // §7 (card `docs/lqos_conf_single_source_card.md`): fold any legacy operator
    // env bandwidth (`bridge.env` + `bridge.env.d/*`) into `lqos.conf`'s
    // `[dataplane]` single source and RETIRE the duplicate env keys. Idempotent,
    // each surface backed up (rollback: `config_migrate::rollback_isp_fold_at`),
    // and NON-FATAL — a failure must never block activation (the legacy env
    // stays authoritative for the bridge's env rung in that case).
    match crate::config_migrate::run_isp_fold() {
        Ok(Some(outcome)) => eprintln!("WARNING: {}", outcome.summary()),
        Ok(None) => {}
        Err(error) => eprintln!(
            "WARNING: the lqos.conf bandwidth fold did not complete ({error}); the legacy env \
             bandwidth remains authoritative for the bridge."
        ),
    }
    // The dataplane orientation must match `lqos.conf`'s pair; the setup/bind
    // moment is the only holder of the netdev->DPDK mapping, so the explicit
    // WAN/LAN BDF pair is written here (idempotent).
    lqos_config::clear_cached_config();
    // lqos-74t.37.7 pin: a broken config must NOT fall through to
    // `Config::default()` — that hides a `[single_interface]` box, skips the
    // topology write, and lets the bridge die at start. Name the load error.
    let config = activation_config(lqos_config::load_config().map_err(|e| e.to_string()))?;
    // The bridge fail-closes when `LIBREQOS_DPDK_CONFIG_FILE` names a missing
    // file; the activation path is its producer (written once, never
    // overwritten). It ALSO writes the `[single_interface]` topology here, so
    // the v2→v3 UPGRADE path gets the same glue as setup success
    // (lqos-74t.37.7). A one-port topology failure is LOUD before any restart.
    if let Some(path) = crate::bootstrap::ensure_bridge_conf_with(&config)? {
        eprintln!(
            "WARNING: wrote/migrated the bridge runtime config {} (the circuit capacity is \
             DERIVED by the emitter; no operator circuit_capacity key; the \
             [single_interface] topology is included when configured).",
            path.display()
        );
    }
    if let Some(path) = crate::bootstrap::ensure_bridge_env_bdf_pair(&config)? {
        eprintln!(
            "WARNING: wrote the explicit WAN/LAN BDF pair into {} \
             (from /etc/libreqos/lqos.conf's pair).",
            path.display()
        );
    }
    // lqos-74t.4.1 (iii): REGENERATE the MANAGED block on an EXISTING box — the
    // managed drop-in (`bridge.env.d/50-ports.conf`: the explicit pair + the EAL
    // order + the bind mode) is rewritten on every activation, so the strongest
    // orientation fact is always present after an UPGRADE that keeps the
    // `bridge.env` conffile (the upgrade hole: a kept env ⇒ no pair ⇒ the
    // positional default ⇒ mis-oriented). Reads the pair just written above.
    // Non-fatal: a failure must never block activation.
    match lqos_setup::port_class::write_plan_from_setup(&config) {
        Ok((_out, msg)) => eprintln!("WARNING: {msg}"),
        Err(e) => {
            eprintln!("WARNING: the managed ports block was not regenerated ({e})")
        }
    }
    // The dataplane package's prerm hands the NICs BACK to the kernel driver,
    // and the bind unit is RemainAfterExit ⇒ a bridge RESTART does not re-run
    // it. Re-bind (idempotent, fail-closed) and clear any latched StartLimit
    // BEFORE starting the bridge — else the bridge crash-loops on DPDK -19 and
    // the post-install runbook is required.
    // lqos-74t.37.11: the bind unit no longer Wants=libreqos-bridge, so the
    // runtime activation must start the bridge EXPLICITLY (below).
    run_systemctl(&["restart", "libreqos-bridge-bind.service"], false)?;
    run_systemctl(&["reset-failed", "libreqos-bridge.service"], false)?;
    // Host CPU power posture (docs/power_profile_work_order.md): enable the
    // boot unit + apply the `[power] profile` now. Bare metal only — a VM
    // prints the host steps and writes nothing. Best-effort: a power failure
    // never blocks the install.
    let _ = run_systemctl(&["enable", "libreqos-power.service"], false);
    lqos_setup::power::apply_at_activation();
    switch_service_mode(
        &["libreqos-first-run.service"],
        RUNTIME_UNITS,
        "Activated runtime services: libreqos-lqosd, libreqos-bridge, libreqos-scheduler, and libreqos-api.",
    )
}

/// Stops runtime services and starts the setup service immediately.
///
/// Side effects: stops and disables `libreqos-lqosd.service` and
/// `libreqos-scheduler.service`, then enables and restarts `libreqos-first-run.service`.
pub(crate) fn activate_setup_service() -> Result<String> {
    switch_service_mode(
        &[
            "libreqos-lqosd.service",
            "libreqos-scheduler.service",
            "libreqos-api.service",
        ],
        &["libreqos-first-run.service"],
        "Activated first-run setup service: libreqos-first-run.",
    )
}

/// Schedules a transient systemd unit that stops setup and starts runtime services.
///
/// Side effects: spawns a transient systemd service, disables and stops
/// `libreqos-first-run.service`, and enables and restarts `libreqos-lqosd.service` and
/// `libreqos-scheduler.service` after a short delay.
pub(crate) fn schedule_runtime_handoff() -> Result<HandoffNotice> {
    if std::env::var_os("INVOCATION_ID").is_none() {
        return Ok(HandoffNotice {
            message: "Setup saved. Start libreqos-lqosd, libreqos-scheduler, and libreqos-api manually because this setup run is not being managed by systemd.".to_string(),
            automatic: false,
        });
    }

    if unsafe { libc::geteuid() } != 0 {
        return Ok(HandoffNotice {
            message: "Setup saved. Start libreqos-lqosd, libreqos-scheduler, and libreqos-api manually because the current setup process is not running as root.".to_string(),
            automatic: false,
        });
    }

    if !std::path::Path::new(SYSTEMD_RUN_BIN).exists() {
        return Ok(HandoffNotice {
            message: "Setup saved. Start libreqos-lqosd, libreqos-scheduler, and libreqos-api manually because systemd-run is not available on this host.".to_string(),
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
            "Unable to schedule runtime handoff via systemd-run (exit status: {status}). Start libreqos-lqosd and libreqos-scheduler manually."
        );
    }

    Ok(HandoffNotice {
        message: format!(
            "Scheduled runtime handoff via transient unit {unit_name}. LibreQoS setup will stop and libreqos-lqosd/libreqos-scheduler/libreqos-api will start shortly."
        ),
        automatic: true,
    })
}

#[cfg(test)]
mod tests {
    use super::{RUNTIME_UNITS, activation_config};

    #[test]
    fn fresh_v3_activation_starts_the_bridge_explicitly() {
        // The bind unit no longer Wants=libreqos-bridge; runtime activation must
        // include it, and start it AFTER lqosd (the boot choreography).
        let pos = |unit: &str| RUNTIME_UNITS.iter().position(|u| *u == unit);
        let lqosd = pos("libreqos-lqosd.service").expect("lqosd present");
        let bridge = pos("libreqos-bridge.service").expect("bridge present");
        let scheduler = pos("libreqos-scheduler.service").expect("scheduler present");
        let api = pos("libreqos-api.service").expect("api present");
        assert!(lqosd < bridge, "lqosd (migration) must precede the bridge");
        assert!(bridge < scheduler, "the scheduler attaches to a live dataplane");
        assert!(scheduler < api, "api follows the scheduler");
    }

    #[test]
    fn an_activation_load_error_is_named_and_never_defaults() {
        let error = activation_config(Err("invalid config fixture".to_string()))
            .expect_err("a load error must stop activation");
        let message = error.to_string();
        assert!(message.contains("invalid config fixture"), "{message}");
        assert!(message.contains("refusing to activate"), "{message}");
    }
}
