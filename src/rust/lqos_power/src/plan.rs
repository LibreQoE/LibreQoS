//! The profile → sysfs plan (pure; `docs/power_profile_work_order.md` §2/§3).
//!
//! `plan` never writes: it reads the host through the [`Sysfs`] seam and
//! returns the ordered settings. Skip any facet the platform lacks.

use crate::platform::{Platform, TurboOff};
use crate::profile::PowerProfile;
use crate::sysfs::Sysfs;

/// The floor/ceiling for the operator-set eco cap (`[power] eco_max_freq_pct`).
pub const ECO_FREQ_CAP_MIN_PCT: u64 = 10;
pub const ECO_FREQ_CAP_MAX_PCT: u64 = 100;
/// States with a resume latency above this (µs) are "deep" — disabled under
/// `performance` (shallow-only, low jitter).
pub const SHALLOW_CSTATE_MAX_LATENCY_US: u64 = 10;

/// Which knob a setting touches (for reporting + tests).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Facet {
    Turbo,
    /// A `scaling_max_freq` cap that implements turbo-off on a platform with
    /// no hard turbo knob (nominal ceiling).
    TurboCap,
    Governor,
    Epp,
    MaxFreq,
    Uncore,
    CState,
}

/// One pending sysfs write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Setting {
    pub facet: Facet,
    pub path: String,
    pub value: String,
}

impl Setting {
    fn new(facet: Facet, path: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            facet,
            path: path.into(),
            value: value.into(),
        }
    }
}

/// The turbo-off ceiling for one policy in kHz: the per-policy
/// `base_frequency` (kHz) when present, otherwise the platform-wide nominal
/// frequency (CPPC, kHz). `None` when the platform exposes neither.
fn policy_nominal_khz(sysfs: &impl Sysfs, scaling_path: &str, platform: &Platform) -> Option<u64> {
    let policy = scaling_path
        .strip_suffix("/scaling_max_freq")
        .unwrap_or(scaling_path);
    sysfs
        .read(&format!("{policy}/base_frequency"))
        .and_then(|v| v.parse::<u64>().ok())
        .or(platform.nominal_freq_khz)
}

/// Build the ordered write plan for `profile` against `platform`.
///
/// **Every** profile forces turbo off first (the hard-default-off gate); the
/// remaining knobs apply only to `eco` / `performance`.
pub fn plan(
    profile: PowerProfile,
    platform: &Platform,
    sysfs: &impl Sysfs,
    eco_max_freq_pct: u64,
) -> Vec<Setting> {
    let mut out = Vec::new();
    let turbo_off = platform.turbo_off();

    // Turbo OFF — always, every profile. `intel_pstate/no_turbo=1` on Intel,
    // `cpufreq/boost=0` on AMD. There is no knob that turns it back on.
    for path in &platform.turbo_paths {
        let value = if path.ends_with("no_turbo") { "1" } else { "0" };
        out.push(Setting::new(Facet::Turbo, path.clone(), value));
    }

    match profile {
        PowerProfile::Balanced => {
            // "turbo off, change nothing else" — except when the platform has
            // no hard turbo knob: then the nominal-frequency cap is the only
            // mechanism that actually enforces turbo-off.
            if matches!(turbo_off, TurboOff::NominalCap(_)) {
                for (scaling, _) in &platform.max_freq_paths {
                    if let Some(nominal) = policy_nominal_khz(sysfs, scaling, platform) {
                        out.push(Setting::new(
                            Facet::TurboCap,
                            scaling.clone(),
                            nominal.to_string(),
                        ));
                    }
                }
            }
        }
        PowerProfile::Eco => {
            for policy in &platform.policies {
                out.push(Setting::new(
                    Facet::Governor,
                    format!("{policy}/scaling_governor"),
                    "powersave",
                ));
            }
            for epp in &platform.epp_paths {
                out.push(Setting::new(Facet::Epp, epp.clone(), "power"));
            }
            let eco_pct = eco_max_freq_pct.clamp(ECO_FREQ_CAP_MIN_PCT, ECO_FREQ_CAP_MAX_PCT);
            for (scaling, cpuinfo) in &platform.max_freq_paths {
                // Eco caps against the turbo-off (nominal) ceiling when the
                // platform exposes it; `cpuinfo_max_freq` includes boost and is
                // only a last-resort fallback.
                let base = policy_nominal_khz(sysfs, scaling, platform)
                    .or_else(|| sysfs.read(cpuinfo).and_then(|v| v.parse::<u64>().ok()))
                    .or_else(|| sysfs.read(scaling).and_then(|v| v.parse::<u64>().ok()));
                if let Some(base) = base {
                    let cap = base.saturating_mul(eco_pct) / 100;
                    out.push(Setting::new(
                        Facet::MaxFreq,
                        scaling.clone(),
                        cap.to_string(),
                    ));
                }
            }
            for dir in &platform.uncore_dirs {
                if let Some(min) = sysfs.read(&format!("{dir}/min_freq_khz")) {
                    out.push(Setting::new(
                        Facet::Uncore,
                        format!("{dir}/max_freq_khz"),
                        min,
                    ));
                }
            }
        }
        PowerProfile::Performance => {
            for policy in &platform.policies {
                out.push(Setting::new(
                    Facet::Governor,
                    format!("{policy}/scaling_governor"),
                    "performance",
                ));
            }
            for epp in &platform.epp_paths {
                out.push(Setting::new(Facet::Epp, epp.clone(), "performance"));
            }
            for dir in &platform.uncore_dirs {
                if let Some(max) = sysfs.read(&format!("{dir}/initial_max_freq_khz")) {
                    out.push(Setting::new(
                        Facet::Uncore,
                        format!("{dir}/max_freq_khz"),
                        max,
                    ));
                }
            }
            // Shallow C-states only: disable every DEEP (high-latency) state.
            for (disable, latency) in &platform.cstate_states {
                if *latency > SHALLOW_CSTATE_MAX_LATENCY_US {
                    out.push(Setting::new(Facet::CState, disable.clone(), "1"));
                }
            }
            // Performance still honors the global turbo-off invariant: keep
            // the nominal cap on a platform without a hard turbo knob.
            if matches!(turbo_off, TurboOff::NominalCap(_)) {
                for (scaling, _) in &platform.max_freq_paths {
                    if let Some(nominal) = policy_nominal_khz(sysfs, scaling, platform) {
                        out.push(Setting::new(
                            Facet::TurboCap,
                            scaling.clone(),
                            nominal.to_string(),
                        ));
                    }
                }
            }
        }
    }

    out
}

/// The exact host-side shell steps for `profile`, printed by `libreqos-setup`
/// when it runs inside a VM (the guest cannot set the host's CPU policy).
pub fn host_steps(profile: PowerProfile) -> Vec<String> {
    let mut steps = vec![
        "# Run these on the HOST (bare metal), not inside the VM.".to_string(),
        "# Freeze the current values first (reversibility):".to_string(),
        "sudo mkdir -p /var/lib/libreqos".to_string(),
        "sudo sh -c 'cat /sys/devices/system/cpu/intel_pstate/no_turbo 2>/dev/null || cat /sys/devices/system/cpu/cpufreq/boost 2>/dev/null' # record the turbo gate".to_string(),
        "# Turbo OFF (always):".to_string(),
        "sudo sh -c 'echo 1 > /sys/devices/system/cpu/intel_pstate/no_turbo 2>/dev/null || echo 0 > /sys/devices/system/cpu/cpufreq/boost 2>/dev/null'".to_string(),
        "# No turbo knob? Cap at the nominal (non-boost) frequency:".to_string(),
        "nom=$(cat /sys/devices/system/cpu/cpu0/acpi_cppc/nominal_freq 2>/dev/null); [ -n \"$nom\" ] && for p in /sys/devices/system/cpu/cpufreq/policy*; do echo $((nom * 1000)) | sudo tee \"$p/scaling_max_freq\"; done".to_string(),
    ];
    match profile {
        PowerProfile::Balanced => {}
        PowerProfile::Eco => {
            steps.push("# Eco: powersave governor + power EPP + a 65% nominal cap.".to_string());
            steps.push("for p in /sys/devices/system/cpu/cpufreq/policy*; do echo powersave | sudo tee \"$p/scaling_governor\"; echo power | sudo tee \"$p/energy_performance_preference\" 2>/dev/null; nom=$(cat /sys/devices/system/cpu/cpu0/acpi_cppc/nominal_freq 2>/dev/null); base=$(cat \"$p/base_frequency\" 2>/dev/null || { [ -n \"$nom\" ] && echo $((nom * 1000)); } || cat \"$p/cpuinfo_max_freq\"); echo $((base * 65 / 100)) | sudo tee \"$p/scaling_max_freq\"; done".to_string());
        }
        PowerProfile::Performance => {
            steps.push(
                "# Performance: performance governor + performance EPP + shallow C-states."
                    .to_string(),
            );
            steps.push("for p in /sys/devices/system/cpu/cpufreq/policy*; do echo performance | sudo tee \"$p/scaling_governor\"; echo performance | sudo tee \"$p/energy_performance_preference\" 2>/dev/null; done".to_string());
            steps.push("for s in /sys/devices/system/cpu/cpu*/cpuidle/state*; do lat=$(cat \"$s/latency\"); [ \"$lat\" -gt 10 ] && echo 1 | sudo tee \"$s/disable\"; done".to_string());
        }
    }
    steps.push(format!(
        "# Then set the profile in the guest's /etc/libreqos/lqos.conf: [power] profile = \"{profile}\"."
    ));
    steps
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{CPU_ROOT, CPUFREQ, INTEL_NO_TURBO, INTEL_UNCORE, TurboOff};
    use crate::sysfs::MemorySysfs;

    fn intel_platform() -> (MemorySysfs, Platform) {
        let mut fs = MemorySysfs::new();
        fs.set(
            &format!("{CPU_ROOT}/cpu0/cpufreq/scaling_driver"),
            "intel_pstate",
        );
        fs.set(INTEL_NO_TURBO, "0");
        fs.set(&format!("{CPUFREQ}/policy0/scaling_governor"), "powersave");
        fs.set(&format!("{CPUFREQ}/policy0/scaling_max_freq"), "5000000");
        fs.set(&format!("{CPUFREQ}/policy0/cpuinfo_max_freq"), "5000000");
        fs.set(
            &format!("{CPUFREQ}/policy0/energy_performance_preference"),
            "balance_performance",
        );
        fs.set(
            &format!("{INTEL_UNCORE}/package_00_die_00/max_freq_khz"),
            "2400000",
        );
        fs.set(
            &format!("{INTEL_UNCORE}/package_00_die_00/min_freq_khz"),
            "1200000",
        );
        fs.set(
            &format!("{INTEL_UNCORE}/package_00_die_00/initial_max_freq_khz"),
            "2400000",
        );
        fs.set(&format!("{CPU_ROOT}/cpu0/cpuidle/state0/latency"), "0");
        fs.set(&format!("{CPU_ROOT}/cpu0/cpuidle/state0/disable"), "0");
        fs.set(&format!("{CPU_ROOT}/cpu1/cpuidle/state1/latency"), "133");
        fs.set(&format!("{CPU_ROOT}/cpu1/cpuidle/state1/disable"), "0");
        fs.dir(CPUFREQ, &["policy0", "boost"]);
        fs.dir(INTEL_UNCORE, &["package_00_die_00"]);
        fs.dir(CPU_ROOT, &["cpu0", "cpu1", "cpufreq", "cpuidle"]);
        fs.dir(&format!("{CPU_ROOT}/cpu0/cpuidle"), &["state0"]);
        fs.dir(&format!("{CPU_ROOT}/cpu1/cpuidle"), &["state1"]);
        let platform = Platform::detect(&fs, &crate::sysfs::NoCommands);
        (fs, platform)
    }

    #[test]
    fn every_profile_forces_turbo_off() {
        let (fs, platform) = intel_platform();
        for profile in PowerProfile::ALL {
            let settings = plan(profile, &platform, &fs, 65);
            assert!(
                settings
                    .iter()
                    .any(|s| s.facet == Facet::Turbo && s.path == INTEL_NO_TURBO && s.value == "1"),
                "{profile} must force turbo off"
            );
        }
    }

    #[test]
    fn balanced_changes_only_turbo() {
        let (fs, platform) = intel_platform();
        let settings = plan(PowerProfile::Balanced, &platform, &fs, 65);
        assert_eq!(settings.len(), 1, "balanced = turbo off only: {settings:?}");
        assert_eq!(settings[0].facet, Facet::Turbo);
    }

    #[test]
    fn eco_caps_freq_and_sets_power_bias() {
        let (fs, platform) = intel_platform();
        let settings = plan(PowerProfile::Eco, &platform, &fs, 65);
        let max = settings
            .iter()
            .find(|s| s.facet == Facet::MaxFreq)
            .expect("eco caps the frequency");
        assert_eq!(max.value, "3250000", "65% of 5 GHz");
        assert!(
            settings
                .iter()
                .any(|s| s.facet == Facet::Governor && s.value == "powersave")
        );
        assert!(
            settings
                .iter()
                .any(|s| s.facet == Facet::Epp && s.value == "power")
        );
    }

    #[test]
    fn performance_disables_only_deep_cstates() {
        let (fs, platform) = intel_platform();
        let settings = plan(PowerProfile::Performance, &platform, &fs, 65);
        let states: Vec<&Setting> = settings
            .iter()
            .filter(|s| s.facet == Facet::CState)
            .collect();
        assert_eq!(states.len(), 1, "only the 133 µs state is deep: {states:?}");
        assert!(states[0].path.ends_with("cpu1/cpuidle/state1/disable"));
    }

    #[test]
    fn missing_facets_are_skipped_not_failed() {
        // A minimal AMD host with only the boost gate: no policies, no EPP, no
        // uncore. The plan must not include those facets.
        let mut fs = MemorySysfs::new();
        fs.set(
            &format!("{CPU_ROOT}/cpu0/cpufreq/scaling_driver"),
            "amd-pstate",
        );
        fs.set(&format!("{CPUFREQ}/boost"), "1");
        fs.dir(CPUFREQ, &["boost"]);
        fs.dir(CPU_ROOT, &["cpu0", "cpufreq"]);
        let platform = Platform::detect(&fs, &crate::sysfs::NoCommands);
        let settings = plan(PowerProfile::Eco, &platform, &fs, 65);
        assert_eq!(settings.len(), 1);
        assert_eq!(settings[0].path, "/sys/devices/system/cpu/cpufreq/boost");
        assert_eq!(settings[0].value, "0");
    }

    /// The .12 shape: amd-pstate-epp with **no** `cpufreq/boost` and no
    /// `base_frequency`, but ACPI CPPC exposes `nominal_freq` (MHz).
    fn amd_epp_platform() -> (MemorySysfs, Platform) {
        let mut fs = MemorySysfs::new();
        fs.set(
            &format!("{CPU_ROOT}/cpu0/cpufreq/scaling_driver"),
            "amd-pstate-epp",
        );
        fs.set(&format!("{CPU_ROOT}/cpu0/acpi_cppc/nominal_freq"), "4501");
        for policy in ["policy0", "policy1"] {
            fs.set(&format!("{CPUFREQ}/{policy}/scaling_governor"), "powersave");
            fs.set(&format!("{CPUFREQ}/{policy}/scaling_max_freq"), "5881000");
            fs.set(&format!("{CPUFREQ}/{policy}/cpuinfo_max_freq"), "5881000");
            fs.set(
                &format!("{CPUFREQ}/{policy}/energy_performance_preference"),
                "power",
            );
        }
        fs.dir(CPUFREQ, &["policy0", "policy1"]);
        fs.dir(CPU_ROOT, &["cpu0", "cpufreq"]);
        let platform = Platform::detect(&fs, &crate::sysfs::NoCommands);
        (fs, platform)
    }

    #[test]
    fn amd_epp_without_a_boost_knob_uses_the_nominal_ceiling() {
        let (fs, platform) = amd_epp_platform();
        assert_eq!(platform.turbo_off(), TurboOff::NominalCap(4_501_000));
        assert_eq!(
            platform.turbo_off().describe(),
            "off (nominal cap 4.501 GHz fallback; no turbo knob)"
        );
        assert!(platform.turbo_paths.is_empty());
        assert_eq!(platform.nominal_freq_khz, Some(4_501_000));
        let _ = fs;
    }

    #[test]
    fn balanced_caps_at_nominal_when_no_turbo_knob_exists() {
        let (fs, platform) = amd_epp_platform();
        let settings = plan(PowerProfile::Balanced, &platform, &fs, 65);
        assert_eq!(settings.len(), 2, "one cap per policy: {settings:?}");
        assert!(settings.iter().all(|s| s.facet == Facet::TurboCap));
        assert!(settings.iter().all(|s| s.value == "4501000"));
    }

    #[test]
    fn eco_caps_at_pct_of_nominal_not_boost_max() {
        let (fs, platform) = amd_epp_platform();
        let settings = plan(PowerProfile::Eco, &platform, &fs, 65);
        let cap = settings
            .iter()
            .find(|s| s.facet == Facet::MaxFreq && s.path.ends_with("policy0/scaling_max_freq"))
            .expect("eco caps policy0");
        assert_eq!(cap.value, "2925650", "65% of nominal 4.501 GHz");
        assert!(
            settings
                .iter()
                .any(|s| s.facet == Facet::Governor && s.value == "powersave")
        );
        assert!(
            settings
                .iter()
                .any(|s| s.facet == Facet::Epp && s.value == "power")
        );
    }

    #[test]
    fn performance_stays_capped_at_nominal() {
        let (fs, platform) = amd_epp_platform();
        let settings = plan(PowerProfile::Performance, &platform, &fs, 65);
        let caps: Vec<&Setting> = settings
            .iter()
            .filter(|s| s.facet == Facet::TurboCap)
            .collect();
        assert_eq!(caps.len(), 2);
        assert!(caps.iter().all(|s| s.value == "4501000"));
        assert!(
            settings
                .iter()
                .any(|s| s.facet == Facet::Governor && s.value == "performance")
        );
    }

    #[test]
    fn no_knob_and_no_nominal_is_explicitly_unsupported() {
        let mut fs = MemorySysfs::new();
        fs.set(
            &format!("{CPU_ROOT}/cpu0/cpufreq/scaling_driver"),
            "amd-pstate-epp",
        );
        fs.set(&format!("{CPUFREQ}/policy0/scaling_max_freq"), "5881000");
        fs.set(&format!("{CPUFREQ}/policy0/cpuinfo_max_freq"), "5881000");
        fs.dir(CPUFREQ, &["policy0"]);
        fs.dir(CPU_ROOT, &["cpu0", "cpufreq"]);
        let platform = Platform::detect(&fs, &crate::sysfs::NoCommands);
        assert_eq!(platform.turbo_off(), TurboOff::Unsupported);
        let balanced = plan(PowerProfile::Balanced, &platform, &fs, 65);
        assert!(
            balanced.is_empty(),
            "balanced has no enforceable turbo-off mechanism: {balanced:?}"
        );
    }
}
