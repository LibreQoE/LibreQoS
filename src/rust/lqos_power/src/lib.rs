//! LibreQoS host CPU power posture (`docs/power_profile_work_order.md`).
//!
//! One config key — `lqos.conf` `[power] profile = eco|balanced|performance` —
//! selects the NON-turbo host knobs. **Turbo is forced OFF for every profile**
//! and there is no knob to re-enable it. `balanced` (the default) changes
//! nothing but turbo. The apply is bare-metal only, conflict-aware, logged, and
//! reversible via `/var/lib/libreqos/power.orig`.

pub mod apply;
pub mod config_edit;
pub mod plan;
pub mod platform;
pub mod profile;
pub mod sysfs;

pub use apply::{AppliedWrite, ApplyReport, apply, restore};
pub use platform::TurboOff;
pub use profile::PowerProfile;
pub use sysfs::{Commands, RealCommands, RealSysfs, Sysfs};

/// Apply the `[power]` settings from the lqos.conf at `config_path` (default
/// `balanced` / 65 % when absent). `force` overrides the A2 conflict REFUSE.
pub fn apply_from_config(
    config_path: &str,
    originals_path: &str,
    dry_run: bool,
    force: bool,
) -> anyhow::Result<ApplyReport> {
    let config = config_edit::read_power_config(config_path).unwrap_or_default();
    let request = apply::ApplyRequest {
        profile: config.profile,
        eco_max_freq_pct: config.eco_max_freq_pct,
        force,
        dry_run,
    };
    apply(&request, &RealSysfs, &RealCommands, originals_path)
}
