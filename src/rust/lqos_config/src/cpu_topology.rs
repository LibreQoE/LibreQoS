//! CPU topology helpers (hybrid P-core/E-core detection).
//!
//! LibreQoS primarily runs on Linux and may be deployed on hybrid CPUs that
//! include both performance cores ("P-cores") and efficiency cores ("E-cores").
//! When possible, we detect E-cores via sysfs and provide a list of CPUs that
//! should be used for shaping / CPU binning.

use crate::Config;
use std::collections::HashSet;
use std::path::Path;
use thiserror::Error;

const POSSIBLE_CPUS_PATH: &str = "/sys/devices/system/cpu/possible";

const CPU_CORE_SYSFS_PATHS: [&str; 2] = [
    "/sys/bus/event_source/devices/cpu_core/cpus",
    "/sys/devices/cpu_core/cpus",
];

const CPU_ATOM_SYSFS_PATHS: [&str; 2] = [
    "/sys/bus/event_source/devices/cpu_atom/cpus",
    "/sys/devices/cpu_atom/cpus",
];

/// Indicates which mechanism was used to choose shaping CPUs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapingCpuSource {
    /// Exclusion is disabled; use all possible CPUs.
    ExclusionDisabled,
    /// Used `/sys/.../cpu_core/cpus` to identify P-cores.
    CpuCoreSysfs,
    /// Used `/sys/.../cpu_atom/cpus` to identify E-cores, then computed `possible - atom`.
    PossibleMinusAtomSysfs,
    /// Hybrid sysfs paths were unavailable; used `/sys/devices/system/cpu/possible`.
    FallbackAllPossible,
    /// Could not read `/sys/devices/system/cpu/possible`; used Rust's available parallelism.
    FallbackAvailableParallelism,
}

/// Result of hybrid CPU detection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShapingCpuDetection {
    /// Whether efficiency core exclusion was requested.
    pub exclude_efficiency_cores: bool,
    /// Source used to determine the shaping CPU list.
    pub source: ShapingCpuSource,
    /// All possible CPU IDs (best effort).
    pub possible: Vec<u32>,
    /// Detected performance-core CPU IDs (may be empty if unknown).
    pub performance: Vec<u32>,
    /// Detected efficiency-core CPU IDs (may be empty if unknown).
    pub efficiency: Vec<u32>,
    /// Final CPU list that should be used for shaping/bins.
    pub shaping: Vec<u32>,
}

/// Errors that can occur when parsing a Linux cpulist string.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum CpuListParseError {
    /// Input was empty or contained no CPU entries.
    #[error("CPU list is empty")]
    Empty,
    /// A numeric component failed to parse.
    #[error("Unable to parse number in CPU list")]
    ParseNumber,
    /// A range was malformed (e.g. `3-1`).
    #[error("Invalid range in CPU list")]
    InvalidRange,
}

fn parse_cpu_list(input: &str) -> Result<Vec<u32>, CpuListParseError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(CpuListParseError::Empty);
    }
    let mut out = Vec::new();
    for part in trimmed.split(',') {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        if let Some((start_s, end_s)) = p.split_once('-') {
            let start = start_s
                .trim()
                .parse::<u32>()
                .map_err(|_| CpuListParseError::ParseNumber)?;
            let end = end_s
                .trim()
                .parse::<u32>()
                .map_err(|_| CpuListParseError::ParseNumber)?;
            if end < start {
                return Err(CpuListParseError::InvalidRange);
            }
            for cpu in start..=end {
                out.push(cpu);
            }
        } else {
            let cpu = p
                .parse::<u32>()
                .map_err(|_| CpuListParseError::ParseNumber)?;
            out.push(cpu);
        }
    }
    if out.is_empty() {
        return Err(CpuListParseError::Empty);
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

fn try_read_cpu_list(path: &str) -> Option<Vec<u32>> {
    if !Path::new(path).exists() {
        return None;
    }
    let raw = std::fs::read_to_string(path).ok()?;
    let list = parse_cpu_list(&raw).ok()?;
    (!list.is_empty()).then_some(list)
}

fn first_cpu_list(paths: &[&str]) -> Option<Vec<u32>> {
    for p in paths.iter().copied() {
        if let Some(list) = try_read_cpu_list(p) {
            return Some(list);
        }
    }
    None
}

fn possible_cpu_list() -> (Vec<u32>, ShapingCpuSource) {
    if let Some(list) = try_read_cpu_list(POSSIBLE_CPUS_PATH) {
        return (list, ShapingCpuSource::FallbackAllPossible);
    }

    let fallback = std::thread::available_parallelism()
        .map(|n| (0..(n.get() as u32)).collect::<Vec<u32>>())
        .unwrap_or_else(|_| vec![0]);
    (fallback, ShapingCpuSource::FallbackAvailableParallelism)
}

/// Detects the set of CPUs that should be used for shaping/CPU binning.
///
/// When `cfg.exclude_efficiency_cores` is enabled, this attempts to identify
/// performance cores via sysfs. If detection fails or yields an empty set,
/// this falls back to "all possible CPUs".
pub fn detect_shaping_cpus(cfg: &Config) -> ShapingCpuDetection {
    let exclude = cfg.exclude_efficiency_cores;

    let (possible, possible_source) = possible_cpu_list();

    if !exclude {
        return ShapingCpuDetection {
            exclude_efficiency_cores: exclude,
            source: ShapingCpuSource::ExclusionDisabled,
            possible: possible.clone(),
            performance: possible.clone(),
            efficiency: Vec::new(),
            shaping: possible,
        };
    }

    let perf_opt = first_cpu_list(&CPU_CORE_SYSFS_PATHS);
    let eff_opt = first_cpu_list(&CPU_ATOM_SYSFS_PATHS);

    // Prefer cpu_core list if present.
    if let Some(mut perf) = perf_opt.clone() {
        // If both are present, remove overlaps just in case.
        if let Some(eff) = eff_opt.clone() {
            if !eff.is_empty() && !perf.is_empty() {
                let eff_set: HashSet<u32> = eff.iter().copied().collect();
                perf.retain(|c| !eff_set.contains(c));
            }
        }

        // Ensure it is a subset of possible CPUs, if that list exists.
        if !possible.is_empty() {
            let poss_set: HashSet<u32> = possible.iter().copied().collect();
            perf.retain(|c| poss_set.contains(c));
        }

        if !perf.is_empty() {
            return ShapingCpuDetection {
                exclude_efficiency_cores: exclude,
                source: ShapingCpuSource::CpuCoreSysfs,
                possible,
                performance: perf.clone(),
                efficiency: eff_opt.unwrap_or_default(),
                shaping: perf,
            };
        }
    }

    // Next-best: if we know efficiency CPUs, compute possible - efficiency.
    if let Some(eff) = eff_opt.clone() {
        if !eff.is_empty() && !possible.is_empty() {
            let eff_set: HashSet<u32> = eff.iter().copied().collect();
            let perf: Vec<u32> = possible
                .iter()
                .copied()
                .filter(|c| !eff_set.contains(c))
                .collect();
            if !perf.is_empty() {
                return ShapingCpuDetection {
                    exclude_efficiency_cores: exclude,
                    source: ShapingCpuSource::PossibleMinusAtomSysfs,
                    possible,
                    performance: perf.clone(),
                    efficiency: eff,
                    shaping: perf,
                };
            }
        }
    }

    // Final fallback: use all possible CPUs.
    ShapingCpuDetection {
        exclude_efficiency_cores: exclude,
        source: possible_source,
        possible: possible.clone(),
        performance: possible.clone(),
        efficiency: eff_opt.unwrap_or_default(),
        shaping: possible,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cpu_list_single() {
        assert_eq!(parse_cpu_list("0").unwrap(), vec![0]);
    }

    #[test]
    fn parse_cpu_list_range() {
        assert_eq!(parse_cpu_list("0-3").unwrap(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn parse_cpu_list_mixed() {
        assert_eq!(
            parse_cpu_list("0-3,8,10-12").unwrap(),
            vec![0, 1, 2, 3, 8, 10, 11, 12]
        );
    }

    #[test]
    fn parse_cpu_list_whitespace_and_dedup() {
        assert_eq!(parse_cpu_list(" 0-1, 1 , 2 \n").unwrap(), vec![0, 1, 2]);
    }

    #[test]
    fn parse_cpu_list_invalid_range() {
        assert_eq!(
            parse_cpu_list("3-1").unwrap_err(),
            CpuListParseError::InvalidRange
        );
    }

    #[test]
    fn parse_cpu_list_empty() {
        assert_eq!(parse_cpu_list(" \n").unwrap_err(), CpuListParseError::Empty);
    }
}
