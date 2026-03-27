//! CPU topology helpers (hybrid P-core/E-core detection).
//!
//! LibreQoS primarily runs on Linux and may be deployed on hybrid CPUs that
//! include both performance cores ("P-cores") and efficiency cores ("E-cores").
//! When enabled, detection probes several Linux and CPU-specific sources in
//! descending order of confidence, persists the resolved split to the LibreQoS
//! runtime directory, and reuses that cached result across processes.

use crate::Config;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::debug;

const POSSIBLE_CPUS_PATH: &str = "/sys/devices/system/cpu/possible";
const CPU_TOPOLOGY_CACHE_FILE: &str = "cpu_topology_cache.json";
const CPU_TOPOLOGY_CACHE_VERSION: u32 = 1;

const CPU_CORE_SYSFS_PATHS: [&str; 2] = [
    "/sys/bus/event_source/devices/cpu_core/cpus",
    "/sys/devices/cpu_core/cpus",
];

const CPU_ATOM_SYSFS_PATHS: [&str; 2] = [
    "/sys/bus/event_source/devices/cpu_atom/cpus",
    "/sys/devices/cpu_atom/cpus",
];

const CORE_TYPE_PERFORMANCE_VALUES: [u32; 2] = [1, 2];
const CORE_TYPE_EFFICIENCY_VALUE: u32 = 3;
const CPU_CAPACITY_EFFICIENCY_PERCENT: u32 = 80;
const CPU_FREQ_EFFICIENCY_PERCENT: u32 = 90;
const CPUID_CORE_TYPE_ATOM: u32 = 0x20;
const CPUID_CORE_TYPE_CORE: u32 = 0x40;

/// Indicates which mechanism was used to choose shaping CPUs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShapingCpuSource {
    /// Exclusion is disabled; use all possible CPUs.
    ExclusionDisabled,
    /// Used `/sys/devices/system/cpu/cpu*/topology/core_type`.
    CoreTypeSysfs,
    /// Used `/sys/devices/system/cpu/cpu*/cpu_capacity`.
    CpuCapacitySysfs,
    /// Used `/sys/.../cpu_core/cpus` to identify P-cores.
    CpuCoreSysfs,
    /// Used `/sys/.../cpu_atom/cpus` to identify E-cores, then computed `possible - atom`.
    PossibleMinusAtomSysfs,
    /// Used Intel hybrid CPUID leaf `0x1A`.
    CpuidHybrid,
    /// Used `cpuinfo_max_freq` as a last-resort heuristic.
    MaxFrequencyHeuristic,
    /// Hybrid detection failed or was not trustworthy; used `/sys/devices/system/cpu/possible`.
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
    /// Whether the resolved topology was loaded from the persisted cache file.
    pub from_cache: bool,
    /// Whether a trustworthy hybrid split was detected.
    pub has_hybrid_split: bool,
    /// Human-readable detail used for logs and tests.
    pub detail: String,
    /// All possible CPU IDs (best effort).
    pub possible: Vec<u32>,
    /// Detected performance-core CPU IDs.
    pub performance: Vec<u32>,
    /// Detected efficiency-core CPU IDs.
    pub efficiency: Vec<u32>,
    /// Final CPU list that should be used for shaping/bins.
    pub shaping: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ResolvedHybridCpuTopology {
    source: ShapingCpuSource,
    from_cache: bool,
    has_hybrid_split: bool,
    detail: String,
    possible: Vec<u32>,
    performance: Vec<u32>,
    efficiency: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CpuTopologyCacheFile {
    version: u32,
    fingerprint: CpuTopologyFingerprint,
    topology: ResolvedHybridCpuTopology,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CpuTopologyFingerprint {
    kernel_release: Option<String>,
    possible: Vec<u32>,
    vendor_id: Option<String>,
    model_name: Option<String>,
    cpu_family: Option<String>,
    model: Option<String>,
}

impl ResolvedHybridCpuTopology {
    fn hybrid(
        source: ShapingCpuSource,
        possible: &[u32],
        performance: Vec<u32>,
        efficiency: Vec<u32>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            source,
            from_cache: false,
            has_hybrid_split: true,
            detail: detail.into(),
            possible: possible.to_vec(),
            performance,
            efficiency,
        }
    }

    fn fallback(source: ShapingCpuSource, possible: &[u32], detail: impl Into<String>) -> Self {
        Self {
            source,
            from_cache: false,
            has_hybrid_split: false,
            detail: detail.into(),
            possible: possible.to_vec(),
            performance: possible.to_vec(),
            efficiency: Vec::new(),
        }
    }
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

fn try_read_u32(path: impl AsRef<Path>) -> Option<u32> {
    std::fs::read_to_string(path)
        .ok()?
        .trim()
        .parse::<u32>()
        .ok()
}

fn read_per_cpu_values(possible: &[u32], suffix: &str) -> BTreeMap<u32, u32> {
    possible
        .iter()
        .filter_map(|cpu| {
            let path = format!("/sys/devices/system/cpu/cpu{cpu}/{suffix}");
            try_read_u32(&path).map(|value| (*cpu, value))
        })
        .collect()
}

fn filter_subset(possible: &[u32], cpus: Vec<u32>) -> Vec<u32> {
    let possible_set: HashSet<u32> = possible.iter().copied().collect();
    let mut filtered: Vec<u32> = cpus
        .into_iter()
        .filter(|cpu| possible_set.contains(cpu))
        .collect();
    filtered.sort_unstable();
    filtered.dedup();
    filtered
}

fn validate_split(
    possible: &[u32],
    performance: Vec<u32>,
    efficiency: Vec<u32>,
) -> Option<(Vec<u32>, Vec<u32>)> {
    let possible_set: HashSet<u32> = possible.iter().copied().collect();
    let mut perf = filter_subset(possible, performance);
    let eff = filter_subset(possible, efficiency);
    if perf.is_empty() || eff.is_empty() {
        return None;
    }

    let eff_set: HashSet<u32> = eff.iter().copied().collect();
    perf.retain(|cpu| !eff_set.contains(cpu));
    if perf.is_empty() {
        return None;
    }

    let union: HashSet<u32> = perf.iter().chain(eff.iter()).copied().collect();
    if union != possible_set {
        return None;
    }

    Some((perf, eff))
}

fn detect_from_core_types(
    possible: &[u32],
    values: &BTreeMap<u32, u32>,
) -> Option<ResolvedHybridCpuTopology> {
    if values.is_empty() {
        return None;
    }

    let performance: Vec<u32> = values
        .iter()
        .filter_map(|(cpu, value)| CORE_TYPE_PERFORMANCE_VALUES.contains(value).then_some(*cpu))
        .collect();
    let efficiency: Vec<u32> = values
        .iter()
        .filter_map(|(cpu, value)| (*value == CORE_TYPE_EFFICIENCY_VALUE).then_some(*cpu))
        .collect();

    let (performance, efficiency) = validate_split(possible, performance, efficiency)?;
    Some(ResolvedHybridCpuTopology::hybrid(
        ShapingCpuSource::CoreTypeSysfs,
        possible,
        performance,
        efficiency,
        "Detected hybrid CPU split via topology/core_type",
    ))
}

fn detect_from_capacity(
    possible: &[u32],
    values: &BTreeMap<u32, u32>,
) -> Option<ResolvedHybridCpuTopology> {
    if values.len() != possible.len() || values.is_empty() {
        return None;
    }

    let max_capacity = values.values().copied().max()?;
    let min_capacity = values.values().copied().min()?;
    if max_capacity == 0 || max_capacity == min_capacity {
        return None;
    }

    let threshold = max_capacity.saturating_mul(CPU_CAPACITY_EFFICIENCY_PERCENT) / 100;
    let mut performance = Vec::new();
    let mut efficiency = Vec::new();
    for (cpu, value) in values {
        if *value < threshold {
            efficiency.push(*cpu);
        } else {
            performance.push(*cpu);
        }
    }

    let (performance, efficiency) = validate_split(possible, performance, efficiency)?;
    Some(ResolvedHybridCpuTopology::hybrid(
        ShapingCpuSource::CpuCapacitySysfs,
        possible,
        performance,
        efficiency,
        format!(
            "Detected hybrid CPU split via cpu_capacity (max={max_capacity}, min={min_capacity})"
        ),
    ))
}

fn detect_from_cpu_core_atom_lists(
    possible: &[u32],
    perf_opt: Option<Vec<u32>>,
    eff_opt: Option<Vec<u32>>,
) -> Option<ResolvedHybridCpuTopology> {
    let filtered_efficiency = eff_opt
        .clone()
        .map(|efficiency| filter_subset(possible, efficiency));

    if let Some(mut performance) = perf_opt {
        performance = filter_subset(possible, performance);
        if let Some(efficiency) = filtered_efficiency.as_ref() {
            let eff_set: HashSet<u32> = efficiency.iter().copied().collect();
            performance.retain(|cpu| !eff_set.contains(cpu));
        }

        if !performance.is_empty() {
            return Some(ResolvedHybridCpuTopology::hybrid(
                ShapingCpuSource::CpuCoreSysfs,
                possible,
                performance,
                filtered_efficiency.unwrap_or_default(),
                "Detected performance CPUs via cpu_core cpulist",
            ));
        }
    }

    if let Some(efficiency) = filtered_efficiency {
        let possible_set: HashSet<u32> = possible.iter().copied().collect();
        let eff_set: HashSet<u32> = efficiency.iter().copied().collect();
        let performance: Vec<u32> = possible_set.difference(&eff_set).copied().collect();
        let (performance, efficiency) = validate_split(possible, performance, efficiency)?;
        return Some(ResolvedHybridCpuTopology::hybrid(
            ShapingCpuSource::PossibleMinusAtomSysfs,
            possible,
            performance,
            efficiency,
            "Detected hybrid CPU split via cpu_atom cpulist",
        ));
    }

    None
}

fn detect_from_frequency(
    possible: &[u32],
    values: &BTreeMap<u32, u32>,
) -> Option<ResolvedHybridCpuTopology> {
    if values.len() != possible.len() || values.is_empty() {
        return None;
    }

    let max_freq = values.values().copied().max()?;
    let min_freq = values.values().copied().min()?;
    if max_freq == 0 || max_freq == min_freq {
        return None;
    }

    let threshold = max_freq.saturating_mul(CPU_FREQ_EFFICIENCY_PERCENT) / 100;
    let mut performance = Vec::new();
    let mut efficiency = Vec::new();
    for (cpu, value) in values {
        if *value < threshold {
            efficiency.push(*cpu);
        } else {
            performance.push(*cpu);
        }
    }

    let (performance, efficiency) = validate_split(possible, performance, efficiency)?;
    Some(ResolvedHybridCpuTopology::hybrid(
        ShapingCpuSource::MaxFrequencyHeuristic,
        possible,
        performance,
        efficiency,
        format!("Detected hybrid CPU split via cpuinfo_max_freq (max={max_freq}, min={min_freq})"),
    ))
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn detect_from_cpuid(possible: &[u32]) -> Option<ResolvedHybridCpuTopology> {
    use nix::sched::{CpuSet, sched_getaffinity, sched_setaffinity};
    use nix::unistd::Pid;

    #[cfg(target_arch = "x86")]
    use std::arch::x86::__cpuid_count;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::__cpuid_count;

    // Safety: CPUID is available on all supported x86/x86_64 platforms, and we query leaf 0 first
    // to determine the maximum supported leaf before using leaf 0x1A.
    if unsafe { __cpuid_count(0, 0) }.eax < 0x1a {
        return None;
    }

    let original_affinity = sched_getaffinity(Pid::from_raw(0)).ok()?;
    let restore_affinity = original_affinity;
    let mut performance = Vec::new();
    let mut efficiency = Vec::new();

    for cpu in possible {
        let mut affinity = CpuSet::new();
        if affinity.set(*cpu as usize).is_err() {
            let _ = sched_setaffinity(Pid::from_raw(0), &restore_affinity);
            return None;
        }
        if sched_setaffinity(Pid::from_raw(0), &affinity).is_err() {
            let _ = sched_setaffinity(Pid::from_raw(0), &restore_affinity);
            return None;
        }

        // Safety: Leaf support is validated above using CPUID leaf 0.
        let leaf = unsafe { __cpuid_count(0x1a, 0) };
        let core_type = (leaf.eax >> 24) & 0xff;
        match core_type {
            CPUID_CORE_TYPE_CORE => performance.push(*cpu),
            CPUID_CORE_TYPE_ATOM => efficiency.push(*cpu),
            _ => {}
        }
    }

    let _ = sched_setaffinity(Pid::from_raw(0), &restore_affinity);

    let (performance, efficiency) = validate_split(possible, performance, efficiency)?;
    Some(ResolvedHybridCpuTopology::hybrid(
        ShapingCpuSource::CpuidHybrid,
        possible,
        performance,
        efficiency,
        "Detected hybrid CPU split via Intel hybrid CPUID leaf 0x1A",
    ))
}

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
fn detect_from_cpuid(_possible: &[u32]) -> Option<ResolvedHybridCpuTopology> {
    None
}

fn detect_hybrid_topology(
    possible: &[u32],
    possible_source: ShapingCpuSource,
) -> ResolvedHybridCpuTopology {
    let core_types = read_per_cpu_values(possible, "topology/core_type");
    if let Some(resolved) = detect_from_core_types(possible, &core_types) {
        return resolved;
    }

    let capacities = read_per_cpu_values(possible, "cpu_capacity");
    if let Some(resolved) = detect_from_capacity(possible, &capacities) {
        return resolved;
    }

    let perf_opt = first_cpu_list(&CPU_CORE_SYSFS_PATHS);
    let eff_opt = first_cpu_list(&CPU_ATOM_SYSFS_PATHS);
    if let Some(resolved) = detect_from_cpu_core_atom_lists(possible, perf_opt, eff_opt) {
        return resolved;
    }

    if let Some(resolved) = detect_from_cpuid(possible) {
        return resolved;
    }

    let frequencies = read_per_cpu_values(possible, "cpufreq/cpuinfo_max_freq");
    if let Some(resolved) = detect_from_frequency(possible, &frequencies) {
        return resolved;
    }

    ResolvedHybridCpuTopology::fallback(
        possible_source,
        possible,
        "No trustworthy hybrid CPU split detected; using all possible CPUs",
    )
}

fn cache_file_path(cfg: &Config) -> PathBuf {
    Path::new(&cfg.lqos_directory).join(CPU_TOPOLOGY_CACHE_FILE)
}

fn read_cpuinfo_identity() -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    let cpuinfo = match std::fs::read_to_string("/proc/cpuinfo") {
        Ok(cpuinfo) => cpuinfo,
        Err(_) => return (None, None, None, None),
    };

    let mut vendor_id = None;
    let mut model_name = None;
    let mut cpu_family = None;
    let mut model = None;
    for line in cpuinfo.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().to_string();
        match key {
            "vendor_id" if vendor_id.is_none() => vendor_id = Some(value),
            "model name" | "Processor" | "Hardware" if model_name.is_none() => {
                model_name = Some(value);
            }
            "cpu family" if cpu_family.is_none() => cpu_family = Some(value),
            "model" if model.is_none() => model = Some(value),
            _ => {}
        }
    }

    (vendor_id, model_name, cpu_family, model)
}

impl CpuTopologyFingerprint {
    fn gather(possible: &[u32]) -> Self {
        let kernel_release = std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let (vendor_id, model_name, cpu_family, model) = read_cpuinfo_identity();
        Self {
            kernel_release,
            possible: possible.to_vec(),
            vendor_id,
            model_name,
            cpu_family,
            model,
        }
    }
}

fn load_topology_cache(
    cfg: &Config,
    fingerprint: &CpuTopologyFingerprint,
) -> Option<ResolvedHybridCpuTopology> {
    let path = cache_file_path(cfg);
    let raw = std::fs::read_to_string(&path).ok()?;
    let cache: CpuTopologyCacheFile = serde_json::from_str(&raw).ok()?;
    if cache.version != CPU_TOPOLOGY_CACHE_VERSION || cache.fingerprint != *fingerprint {
        return None;
    }

    let mut topology = cache.topology;
    topology.from_cache = true;
    Some(topology)
}

fn store_topology_cache(
    cfg: &Config,
    fingerprint: &CpuTopologyFingerprint,
    topology: &ResolvedHybridCpuTopology,
) {
    let path = cache_file_path(cfg);
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        debug!("Unable to create CPU topology cache directory {parent:?}: {e}");
        return;
    }

    let temp_path = path.with_extension("json.tmp");
    let cache = CpuTopologyCacheFile {
        version: CPU_TOPOLOGY_CACHE_VERSION,
        fingerprint: fingerprint.clone(),
        topology: ResolvedHybridCpuTopology {
            from_cache: false,
            ..topology.clone()
        },
    };

    let serialized = match serde_json::to_string_pretty(&cache) {
        Ok(serialized) => serialized,
        Err(e) => {
            debug!("Unable to serialize CPU topology cache: {e}");
            return;
        }
    };

    if let Err(e) = std::fs::write(&temp_path, serialized.as_bytes()) {
        debug!("Unable to write temporary CPU topology cache {temp_path:?}: {e}");
        return;
    }

    if let Err(e) = std::fs::rename(&temp_path, &path) {
        debug!("Unable to atomically replace CPU topology cache {path:?}: {e}");
        let _ = std::fs::remove_file(&temp_path);
    }
}

fn cached_or_detected_topology(cfg: &Config) -> ResolvedHybridCpuTopology {
    let (possible, possible_source) = possible_cpu_list();
    let fingerprint = CpuTopologyFingerprint::gather(&possible);

    load_topology_cache(cfg, &fingerprint).unwrap_or_else(|| {
        let detected = detect_hybrid_topology(&possible, possible_source);
        store_topology_cache(cfg, &fingerprint, &detected);
        detected
    })
}

/// Detects the set of CPUs that should be used for shaping/CPU binning.
///
/// When `cfg.exclude_efficiency_cores` is enabled, this may read Linux sysfs,
/// CPU information, and a cached topology file under the LibreQoS runtime
/// directory. If hybrid detection fails or yields an untrustworthy split,
/// LibreQoS falls back to all possible CPUs so non-hybrid systems are
/// unaffected.
pub fn detect_shaping_cpus(cfg: &Config) -> ShapingCpuDetection {
    let exclude = cfg.exclude_efficiency_cores;

    if !exclude {
        let (possible, _) = possible_cpu_list();
        return ShapingCpuDetection {
            exclude_efficiency_cores: false,
            source: ShapingCpuSource::ExclusionDisabled,
            from_cache: false,
            has_hybrid_split: false,
            detail: "Efficiency-core exclusion disabled".to_string(),
            possible: possible.clone(),
            performance: possible.clone(),
            efficiency: Vec::new(),
            shaping: possible,
        };
    }

    let topology = cached_or_detected_topology(cfg);
    let performance = if topology.has_hybrid_split {
        topology.performance.clone()
    } else {
        topology.possible.clone()
    };
    let efficiency = if topology.has_hybrid_split {
        topology.efficiency.clone()
    } else {
        Vec::new()
    };

    ShapingCpuDetection {
        exclude_efficiency_cores: true,
        source: topology.source,
        from_cache: topology.from_cache,
        has_hybrid_split: topology.has_hybrid_split,
        detail: topology.detail,
        possible: topology.possible.clone(),
        performance: performance.clone(),
        efficiency,
        shaping: performance,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(name: &str) -> PathBuf {
        let unique = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("libreqos-{name}-{unique}"));
        std::fs::create_dir_all(&dir).expect("test temp dir should be creatable");
        dir
    }

    #[test]
    fn parse_cpu_list_single() {
        assert_eq!(
            parse_cpu_list("0").expect("single CPU list should parse"),
            vec![0]
        );
    }

    #[test]
    fn parse_cpu_list_range() {
        assert_eq!(
            parse_cpu_list("0-3").expect("CPU range should parse"),
            vec![0, 1, 2, 3]
        );
    }

    #[test]
    fn parse_cpu_list_mixed() {
        assert_eq!(
            parse_cpu_list("0-3,8,10-12").expect("mixed CPU list should parse"),
            vec![0, 1, 2, 3, 8, 10, 11, 12]
        );
    }

    #[test]
    fn parse_cpu_list_whitespace_and_dedup() {
        assert_eq!(
            parse_cpu_list(" 0-1, 1 , 2 \n").expect("whitespace-delimited CPU list should parse"),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn parse_cpu_list_invalid_range() {
        assert_eq!(
            parse_cpu_list("3-1").expect_err("reversed CPU range should fail"),
            CpuListParseError::InvalidRange
        );
    }

    #[test]
    fn parse_cpu_list_empty() {
        assert_eq!(
            parse_cpu_list(" \n").expect_err("empty CPU list should fail"),
            CpuListParseError::Empty
        );
    }

    #[test]
    fn detects_core_type_split() {
        let possible = vec![0, 1, 2, 3];
        let values = BTreeMap::from([(0, 1), (1, 2), (2, 3), (3, 3)]);
        let detection =
            detect_from_core_types(&possible, &values).expect("core_type should detect hybrid");
        assert_eq!(detection.source, ShapingCpuSource::CoreTypeSysfs);
        assert_eq!(detection.performance, vec![0, 1]);
        assert_eq!(detection.efficiency, vec![2, 3]);
    }

    #[test]
    fn detects_cpu_capacity_split() {
        let possible = vec![0, 1, 2, 3];
        let values = BTreeMap::from([(0, 1024), (1, 1024), (2, 512), (3, 512)]);
        let detection =
            detect_from_capacity(&possible, &values).expect("cpu_capacity should detect hybrid");
        assert_eq!(detection.source, ShapingCpuSource::CpuCapacitySysfs);
        assert_eq!(detection.performance, vec![0, 1]);
        assert_eq!(detection.efficiency, vec![2, 3]);
    }

    #[test]
    fn capacity_rejects_non_partitioned_split() {
        let possible = vec![0, 1, 2, 3];
        let values = BTreeMap::from([(0, 1024), (1, 1024), (2, 512)]);
        assert!(
            detect_from_capacity(&possible, &values).is_none(),
            "partial capacity data should not be trusted"
        );
    }

    #[test]
    fn detects_cpu_core_atom_lists() {
        let possible = vec![0, 1, 2, 3];
        let detection =
            detect_from_cpu_core_atom_lists(&possible, Some(vec![0, 1]), Some(vec![2, 3]))
                .expect("cpu_core/cpu_atom should detect hybrid");
        assert_eq!(detection.source, ShapingCpuSource::CpuCoreSysfs);
        assert_eq!(detection.performance, vec![0, 1]);
        assert_eq!(detection.efficiency, vec![2, 3]);
    }

    #[test]
    fn detects_cpu_core_list_without_atom_list() {
        let possible = vec![0, 1, 2, 3];
        let detection = detect_from_cpu_core_atom_lists(&possible, Some(vec![0, 1]), None)
            .expect("cpu_core without cpu_atom should still detect performance CPUs");
        assert_eq!(detection.source, ShapingCpuSource::CpuCoreSysfs);
        assert!(detection.has_hybrid_split);
        assert_eq!(detection.performance, vec![0, 1]);
        assert!(detection.efficiency.is_empty());
    }

    #[test]
    fn detects_frequency_split() {
        let possible = vec![0, 1, 2, 3];
        let values = BTreeMap::from([(0, 5200000), (1, 5200000), (2, 3900000), (3, 3900000)]);
        let detection = detect_from_frequency(&possible, &values)
            .expect("frequency split should detect hybrid");
        assert_eq!(detection.source, ShapingCpuSource::MaxFrequencyHeuristic);
        assert_eq!(detection.performance, vec![0, 1]);
        assert_eq!(detection.efficiency, vec![2, 3]);
    }

    #[test]
    fn fallback_uses_all_possible_cpus() {
        let possible = vec![9_999];
        let detection = detect_hybrid_topology(&possible, ShapingCpuSource::FallbackAllPossible);
        assert_eq!(detection.source, ShapingCpuSource::FallbackAllPossible);
        assert!(!detection.has_hybrid_split);
        assert_eq!(detection.performance, possible);
        assert!(detection.efficiency.is_empty());
    }

    #[test]
    fn topology_cache_round_trip() {
        let cfg = Config {
            lqos_directory: temp_dir("cpu-topology-cache").display().to_string(),
            ..Config::default()
        };
        let path = cache_file_path(&cfg);
        let fingerprint = CpuTopologyFingerprint {
            kernel_release: Some("6.9.0".to_string()),
            possible: vec![0, 1, 2, 3],
            vendor_id: Some("GenuineIntel".to_string()),
            model_name: Some("Test CPU".to_string()),
            cpu_family: Some("6".to_string()),
            model: Some("151".to_string()),
        };
        let topology = ResolvedHybridCpuTopology::hybrid(
            ShapingCpuSource::CoreTypeSysfs,
            &[0, 1, 2, 3],
            vec![0, 1],
            vec![2, 3],
            "test topology",
        );

        store_topology_cache(&cfg, &fingerprint, &topology);
        let loaded = load_topology_cache(&cfg, &fingerprint).expect("cache should round-trip");
        assert!(loaded.from_cache);
        assert_eq!(loaded.source, topology.source);
        assert_eq!(loaded.performance, topology.performance);
        assert_eq!(loaded.efficiency, topology.efficiency);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn topology_cache_invalidates_on_fingerprint_change() {
        let cfg = Config {
            lqos_directory: temp_dir("cpu-topology-cache-mismatch")
                .display()
                .to_string(),
            ..Config::default()
        };
        let path = cache_file_path(&cfg);
        let fingerprint = CpuTopologyFingerprint {
            kernel_release: Some("6.9.0".to_string()),
            possible: vec![0, 1, 2, 3],
            vendor_id: Some("GenuineIntel".to_string()),
            model_name: Some("Test CPU".to_string()),
            cpu_family: Some("6".to_string()),
            model: Some("151".to_string()),
        };
        let topology = ResolvedHybridCpuTopology::hybrid(
            ShapingCpuSource::CoreTypeSysfs,
            &[0, 1, 2, 3],
            vec![0, 1],
            vec![2, 3],
            "test topology",
        );
        store_topology_cache(&cfg, &fingerprint, &topology);

        let mut mismatched = fingerprint.clone();
        mismatched.kernel_release = Some("6.10.0".to_string());
        assert!(
            load_topology_cache(&cfg, &mismatched).is_none(),
            "fingerprint mismatch should invalidate the cache"
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cached_detection_uses_each_config_directory_independently() {
        let (possible, possible_source) = possible_cpu_list();
        let fingerprint = CpuTopologyFingerprint::gather(&possible);

        let cfg_a = Config {
            lqos_directory: temp_dir("cpu-topology-cache-a").display().to_string(),
            ..Config::default()
        };
        let cfg_b = Config {
            lqos_directory: temp_dir("cpu-topology-cache-b").display().to_string(),
            ..Config::default()
        };

        let path_a = cache_file_path(&cfg_a);
        let path_b = cache_file_path(&cfg_b);

        let topology_a = ResolvedHybridCpuTopology::fallback(
            possible_source,
            &possible,
            "cache entry from directory a",
        );
        let topology_b = ResolvedHybridCpuTopology::fallback(
            ShapingCpuSource::FallbackAvailableParallelism,
            &possible,
            "cache entry from directory b",
        );

        store_topology_cache(&cfg_a, &fingerprint, &topology_a);
        store_topology_cache(&cfg_b, &fingerprint, &topology_b);

        let loaded_a = cached_or_detected_topology(&cfg_a);
        let loaded_b = cached_or_detected_topology(&cfg_b);

        assert!(loaded_a.from_cache);
        assert!(loaded_b.from_cache);
        assert_eq!(loaded_a.detail, "cache entry from directory a");
        assert_eq!(loaded_b.detail, "cache entry from directory b");
        assert_eq!(loaded_a.source, possible_source);
        assert_eq!(
            loaded_b.source,
            ShapingCpuSource::FallbackAvailableParallelism
        );

        let _ = std::fs::remove_file(path_a);
        let _ = std::fs::remove_file(path_b);
    }
}
