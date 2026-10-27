//! Platform detection: bare metal vs VM, the cpufreq driver, and which facets
//! the host actually exposes (`docs/power_profile_work_order.md` §3).
//!
//! The rule is **skip anything the platform does not expose — never fail the
//! install**: an absent facet is simply not in the plan.

use crate::sysfs::{Commands, Sysfs};

pub const CPU_ROOT: &str = "/sys/devices/system/cpu";
pub const CPUFREQ: &str = "/sys/devices/system/cpu/cpufreq";
pub const INTEL_NO_TURBO: &str = "/sys/devices/system/cpu/intel_pstate/no_turbo";
pub const AMD_BOOST: &str = "/sys/devices/system/cpu/cpufreq/boost";
pub const INTEL_UNCORE: &str = "/sys/devices/system/cpu/intel_uncore_frequency";

/// One probe of the host's power-relevant sysfs surfaces.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Platform {
    pub virtualized: bool,
    /// `intel_pstate` / `amd-pstate-epp` / `amd-pstate` / `acpi-cpufreq` / …
    pub scaling_driver: Option<String>,
    /// Turbo gates present: `intel_pstate/no_turbo` and/or `cpufreq/boost`.
    pub turbo_paths: Vec<String>,
    /// `/sys/devices/system/cpu/cpufreq/policyN`.
    pub policies: Vec<String>,
    /// `policyN/energy_performance_preference` (EPP mode only).
    pub epp_paths: Vec<String>,
    /// `(policyN/scaling_max_freq, policyN/cpuinfo_max_freq)`.
    pub max_freq_paths: Vec<(String, String)>,
    /// Nominal (non-boost) frequency in kHz, when the platform exposes it.
    /// Sources, in order: `policyN/base_frequency` (kHz) or
    /// `cpu0/acpi_cppc/nominal_freq` (MHz, converted here).
    pub nominal_freq_khz: Option<u64>,
    /// `intel_uncore_frequency/package_*_die_*` dirs that expose `max_freq_khz`.
    pub uncore_dirs: Vec<String>,
    /// `(cpuN/cpuidle/stateM/disable, latency_us)` for every idle state.
    pub cstate_states: Vec<(String, u64)>,
}

/// How this platform can enforce the product's "turbo is always off" gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurboOff {
    /// `intel_pstate/no_turbo=1`.
    NoTurbo,
    /// `cpufreq/boost=0`.
    Boost,
    /// No hard turbo knob: cap `scaling_max_freq` at the nominal ceiling.
    NominalCap(u64),
    /// No hard knob and no nominal ceiling: turbo-off cannot be enforced.
    Unsupported,
}

impl TurboOff {
    /// A short, unambiguous status line (never `turbo=0`, which used to mean
    /// "zero turbo *paths* detected" and read like "turbo is off").
    pub fn describe(self) -> String {
        match self {
            Self::NoTurbo => "off (intel_pstate/no_turbo=1)".to_string(),
            Self::Boost => "off (cpufreq/boost=0)".to_string(),
            Self::NominalCap(khz) => format!(
                "off (nominal cap {:.3} GHz fallback; no turbo knob)",
                khz as f64 / 1_000_000.0
            ),
            Self::Unsupported => "UNSUPPORTED (no turbo knob and no nominal frequency; \
                 the profile cannot enforce turbo-off)"
                .to_string(),
        }
    }
}

impl Platform {
    /// Detect from the seams. Pure — every path is read through `sysfs`/`cmds`.
    pub fn detect(sysfs: &impl Sysfs, cmds: &impl Commands) -> Self {
        let virtualized = match cmds.run("systemd-detect-virt", &["--quiet"]) {
            Some((code, _)) => code == 0,
            // No systemd-detect-virt: fall back to the CPU hypervisor flag.
            None => sysfs
                .read("/proc/cpuinfo")
                .is_some_and(|cpu| cpu.contains(" hypervisor")),
        };

        let scaling_driver = sysfs.read(&format!("{CPU_ROOT}/cpu0/cpufreq/scaling_driver"));

        let mut turbo_paths = Vec::new();
        if sysfs.exists(INTEL_NO_TURBO) {
            turbo_paths.push(INTEL_NO_TURBO.to_string());
        }
        if sysfs.exists(AMD_BOOST) {
            turbo_paths.push(AMD_BOOST.to_string());
        }

        let mut policies: Vec<String> = sysfs
            .read_dir(CPUFREQ)
            .into_iter()
            .filter(|e| e.starts_with("policy"))
            .map(|e| format!("{CPUFREQ}/{e}"))
            .collect();
        policies.sort();

        let mut epp_paths = Vec::new();
        let mut max_freq_paths = Vec::new();
        for policy in &policies {
            let epp = format!("{policy}/energy_performance_preference");
            if sysfs.exists(&epp) {
                epp_paths.push(epp);
            }
            let scaling = format!("{policy}/scaling_max_freq");
            if sysfs.exists(&scaling) {
                max_freq_paths.push((scaling, format!("{policy}/cpuinfo_max_freq")));
            }
        }

        // The turbo-off ceiling when the platform has no hard turbo knob.
        // amd-pstate-epp exposes the nominal (non-boost) frequency through
        // ACPI CPPC in MHz; Intel/acpi-cpufreq may expose base_frequency in
        // kHz. Prefer the per-policy value, then the platform-wide CPPC one.
        let mut nominal_freq_khz = policies
            .iter()
            .find_map(|policy| sysfs.read(&format!("{policy}/base_frequency")))
            .and_then(|v| v.parse::<u64>().ok());
        if nominal_freq_khz.is_none() {
            nominal_freq_khz = sysfs
                .read(&format!("{CPU_ROOT}/cpu0/acpi_cppc/nominal_freq"))
                .and_then(|v| v.parse::<u64>().ok())
                .map(|mhz| mhz.saturating_mul(1000));
        }

        let uncore_dirs: Vec<String> = sysfs
            .read_dir(INTEL_UNCORE)
            .into_iter()
            .filter(|e| e.starts_with("package"))
            .map(|e| format!("{INTEL_UNCORE}/{e}"))
            .filter(|dir| sysfs.exists(&format!("{dir}/max_freq_khz")))
            .collect();

        // Idle states across every CPU (cpuidle is per-CPU).
        let mut cstate_states = Vec::new();
        let mut cpus: Vec<String> = sysfs
            .read_dir(CPU_ROOT)
            .into_iter()
            .filter(|e| {
                e.strip_prefix("cpu")
                    .is_some_and(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_digit()))
            })
            .collect();
        cpus.sort();
        for cpu in cpus {
            let idle_dir = format!("{CPU_ROOT}/{cpu}/cpuidle");
            let mut states: Vec<String> = sysfs
                .read_dir(&idle_dir)
                .into_iter()
                .filter(|e| e.starts_with("state"))
                .collect();
            states.sort();
            for state in states {
                let base = format!("{idle_dir}/{state}");
                let disable = format!("{base}/disable");
                if !sysfs.exists(&disable) {
                    continue;
                }
                let latency = sysfs
                    .read(&format!("{base}/latency"))
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0);
                cstate_states.push((disable, latency));
            }
        }

        Self {
            virtualized,
            scaling_driver,
            turbo_paths,
            policies,
            epp_paths,
            max_freq_paths,
            nominal_freq_khz,
            uncore_dirs,
            cstate_states,
        }
    }

    /// The strongest turbo-off mechanism this host exposes.
    pub fn turbo_off(&self) -> TurboOff {
        if self.turbo_paths.iter().any(|p| p.ends_with("no_turbo")) {
            TurboOff::NoTurbo
        } else if self.turbo_paths.iter().any(|p| p.ends_with("boost")) {
            TurboOff::Boost
        } else if let Some(khz) = self.nominal_freq_khz {
            TurboOff::NominalCap(khz)
        } else {
            TurboOff::Unsupported
        }
    }
}
