//! Node inventory endpoint.
//!
//! `/version` reports the installed LibreQoS / dataplane packages, the bridge
//! and `lqosd` build metadata, and this API's own build. Every probe is
//! best-effort: a missing component is reported as `null` plus an entry in
//! `unavailable`, and the endpoint always answers 200.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::Query;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use utoipa::ToSchema;

const SCHEMA_VERSION: u32 = 1;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
const CACHE_TTL: Duration = Duration::from_secs(60);
const BRIDGE_MIN_VERSION: VersionCore = VersionCore(3, 0, 7);
const DPKG_QUERY: &str = "dpkg-query";
const BRIDGE_BIN: &str = "/usr/libexec/libreqos/bridge_rs";
const BRIDGE_LIB_DIR: &str = "/usr/libexec/libreqos/lib";
const LQOSD_BIN: &str = "/usr/libexec/libreqos/lqosd";

#[derive(Clone, Debug, Serialize, ToSchema, PartialEq)]
pub struct VersionReport {
    /// Inventory schema version (currently 1).
    pub schema_version: u32,
    /// Host name of the node.
    pub hostname: String,
    /// The control-plane package, or null when it is not installed.
    pub libreqos: Option<LibreqosVersion>,
    /// The dataplane package, or null when it is not installed.
    pub dataplane: Option<DataplaneVersion>,
    /// The running daemon's protocol, or null when it cannot be read.
    pub lqosd: Option<LqosdVersion>,
    /// This API's version and build revision.
    pub lqos_api: LqosApiVersion,
    /// Probe time, UTC, RFC 3339.
    pub generated_at: String,
    /// Sorted dotted keys for every component/value that could not be read.
    pub unavailable: Vec<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema, PartialEq)]
pub struct LibreqosVersion {
    pub package: String,
    pub version: String,
    /// The package version with a trailing `.YYYYmmddHHMM` stamp removed.
    pub source_version: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema, PartialEq)]
pub struct DataplaneVersion {
    pub package: String,
    pub version: String,
    /// Bridge build revision, `"unknown"`, or null when unavailable.
    pub build_rev: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema, PartialEq)]
pub struct LqosdVersion {
    pub protocol: u32,
}

#[derive(Clone, Debug, Serialize, ToSchema, PartialEq)]
pub struct LqosApiVersion {
    pub version: String,
    /// Lower-case 12-hex git revision, or `"unknown"`.
    pub build_rev: String,
}

/// Injectable command paths and clock used by the probe (tests use fakes).
pub(crate) struct VersionProbe {
    pub dpkg_query: PathBuf,
    pub bridge_bin: PathBuf,
    pub bridge_library_path: PathBuf,
    pub lqosd_bin: PathBuf,
    pub timeout: Duration,
    pub now: Arc<dyn Fn() -> SystemTime + Send + Sync>,
}

impl Default for VersionProbe {
    fn default() -> Self {
        Self {
            dpkg_query: PathBuf::from(DPKG_QUERY),
            bridge_bin: PathBuf::from(BRIDGE_BIN),
            bridge_library_path: PathBuf::from(BRIDGE_LIB_DIR),
            lqosd_bin: PathBuf::from(LQOSD_BIN),
            timeout: DEFAULT_TIMEOUT,
            now: Arc::new(SystemTime::now),
        }
    }
}

/// The single-flight cache around the blocking probes.
pub(crate) struct VersionService {
    probe: Arc<VersionProbe>,
    ttl: Duration,
    cache: Mutex<Option<(Instant, VersionReport)>>,
}

impl Default for VersionService {
    fn default() -> Self {
        Self::with_probe(VersionProbe::default(), CACHE_TTL)
    }
}

impl VersionService {
    pub(crate) fn with_probe(probe: VersionProbe, ttl: Duration) -> Self {
        Self {
            probe: Arc::new(probe),
            ttl,
            cache: Mutex::new(None),
        }
    }

    pub(crate) async fn report(&self) -> VersionReport {
        let mut cache = self.cache.lock().await;
        if let Some((at, report)) = cache.as_ref()
            && at.elapsed() < self.ttl
        {
            return report.clone();
        }

        let probe = Arc::clone(&self.probe);
        let fallback = Arc::clone(&self.probe);
        let report = match tokio::task::spawn_blocking(move || probe_version(&probe)).await {
            Ok(report) => report,
            // A panic/abort in the blocking task is still a partial 200.
            Err(_) => fallback_report(&fallback),
        };
        *cache = Some((Instant::now(), report.clone()));
        report
    }
}

static VERSION_SERVICE: Lazy<VersionService> = Lazy::new(VersionService::default);

/// Test-only lab hooks, enabled with `LQOS_API_TEST_HOOKS=1`.
///
/// The hook exists for the incident-matrix harness and is inert in every normal
/// deployment: `?simulate_missing=bridge` forces the bridge build-rev probe
/// result unavailable.
pub(crate) fn test_hooks_enabled() -> bool {
    std::env::var("LQOS_API_TEST_HOOKS").as_deref() == Ok("1")
}

#[derive(Debug, Default, Deserialize)]
pub struct VersionQuery {
    /// Test-only: `bridge` forces `dataplane.build_rev` unavailable.
    simulate_missing: Option<String>,
}

#[utoipa::path(
    get,
    path = "/version",
    summary = "Node package, dataplane, daemon, and API versions",
    description = "Return the installed libreqos/libreqos-dataplane package versions, the dataplane bridge build revision, the lqosd protocol version, and this API's version. Missing components are null and listed in `unavailable`; the endpoint always returns 200 to an authenticated caller.",
    responses(
        (status = 200, description = "Version inventory (partial data is returned with `unavailable` keys).", body = VersionReport),
        (status = 401, description = "Return 401 Unauthorized if the local API key or license key is missing or invalid.", body = String),
    ),
    tag = "Health & Status",
    params(
        ("x-bearer" = String, Header, description = "A named or legacy local API key, or license key, to use for authentication.", example = "lqos_api_<uuid>_<secret>"),
    ),
)]
pub async fn version(
    Query(query): Query<VersionQuery>,
) -> (axum::http::StatusCode, axum::Json<VersionReport>) {
    let mut report = VERSION_SERVICE.report().await;
    if test_hooks_enabled() && query.simulate_missing.as_deref() == Some("bridge") {
        if let Some(dataplane) = report.dataplane.as_mut() {
            dataplane.build_rev = None;
        }
        report.unavailable.push("dataplane.build_rev".to_string());
        report.unavailable.sort();
        report.unavailable.dedup();
    }
    (axum::http::StatusCode::OK, axum::Json(report))
}

fn probe_version(probe: &VersionProbe) -> VersionReport {
    let packages = query_packages(probe);
    let libreqos_version = packages.get("libreqos").cloned();
    let dataplane_version = packages.get("libreqos-dataplane").cloned();
    let mut unavailable = Vec::new();

    let libreqos = libreqos_version.as_ref().map(|version| {
        let source_version = strip_source_version(version);
        if source_version.is_none() {
            unavailable.push("libreqos.source_version".to_string());
        }
        LibreqosVersion {
            package: "libreqos".to_string(),
            version: version.clone(),
            source_version,
        }
    });
    if libreqos.is_none() {
        unavailable.push("libreqos".to_string());
        unavailable.push("libreqos.source_version".to_string());
    }

    let dataplane = dataplane_version.map(|version| {
        let build_rev = if version_at_least(&version, BRIDGE_MIN_VERSION) {
            probe_bridge(probe)
        } else {
            None
        };
        if !matches!(
            build_rev.as_deref(),
            Some(rev) if is_hex12(rev) || rev == "unknown"
        ) {
            unavailable.push("dataplane.build_rev".to_string());
        }
        DataplaneVersion {
            package: "libreqos-dataplane".to_string(),
            version,
            build_rev,
        }
    });
    if dataplane.is_none() {
        unavailable.push("dataplane".to_string());
        unavailable.push("dataplane.build_rev".to_string());
    }

    let lqosd = match libreqos_version
        .as_deref()
        .filter(|version| version_at_least(version, BRIDGE_MIN_VERSION))
    {
        Some(_) => match probe_lqosd(probe) {
            Some(protocol) => Some(LqosdVersion { protocol }),
            None => {
                unavailable.push("lqosd".to_string());
                None
            }
        },
        None => {
            unavailable.push("lqosd".to_string());
            None
        }
    };

    let build_rev = api_build_rev();
    if build_rev == "unknown" {
        unavailable.push("lqos_api.build_rev".to_string());
    }
    unavailable.sort();
    unavailable.dedup();

    VersionReport {
        schema_version: SCHEMA_VERSION,
        hostname: hostname(),
        libreqos,
        dataplane,
        lqosd,
        lqos_api: LqosApiVersion {
            version: env!("CARGO_PKG_VERSION").to_string(),
            build_rev,
        },
        generated_at: rfc3339_utc((probe.now)()),
        unavailable,
    }
}

fn fallback_report(probe: &VersionProbe) -> VersionReport {
    let build_rev = api_build_rev();
    let mut unavailable = vec![
        "libreqos".to_string(),
        "libreqos.source_version".to_string(),
        "dataplane".to_string(),
        "dataplane.build_rev".to_string(),
        "lqosd".to_string(),
    ];
    if build_rev == "unknown" {
        unavailable.push("lqos_api.build_rev".to_string());
    }
    unavailable.sort();
    unavailable.dedup();
    VersionReport {
        schema_version: SCHEMA_VERSION,
        hostname: hostname(),
        libreqos: None,
        dataplane: None,
        lqosd: None,
        lqos_api: LqosApiVersion {
            version: env!("CARGO_PKG_VERSION").to_string(),
            build_rev,
        },
        generated_at: rfc3339_utc((probe.now)()),
        unavailable,
    }
}

fn query_packages(probe: &VersionProbe) -> HashMap<String, String> {
    let mut command = Command::new(&probe.dpkg_query);
    // Fixed argv, no shell. The literal tab/newline are the format's own
    // separators; dpkg-query sees them as part of the format string.
    command.args([
        "-W",
        "-f=${Package}\t${Version}\n",
        "libreqos",
        "libreqos-dataplane",
    ]);
    // A missing package makes dpkg-query exit 1 while still printing the
    // packages it did find, so do not require success here.
    let Some(output) = run_capture(command, probe.timeout, false) else {
        return HashMap::new();
    };
    let mut packages = HashMap::new();
    for line in output.lines() {
        let mut fields = line.split_whitespace();
        if let (Some(package), Some(version)) = (fields.next(), fields.next()) {
            packages.insert(package.to_string(), version.to_string());
        }
    }
    packages
}

fn probe_bridge(probe: &VersionProbe) -> Option<String> {
    let mut command = Command::new(&probe.bridge_bin);
    command
        .arg("--print-build-rev")
        .env("LD_LIBRARY_PATH", &probe.bridge_library_path);
    let output = run_capture(command, probe.timeout, true)?;
    let rev = output.trim();
    if is_hex12(rev) || rev == "unknown" {
        Some(rev.to_string())
    } else {
        None
    }
}

fn probe_lqosd(probe: &VersionProbe) -> Option<u32> {
    lqosd_protocol(&probe.lqosd_bin, probe.timeout)
}

/// Read the installed `lqosd`'s wire protocol version (its
/// `--print-protocol-version` probe), bounded by `timeout`.
///
/// Shared by the `/version` inventory probe and the startup fail-closed
/// protocol check (`crate::protocol`): one implementation, one timeout.
pub(crate) fn lqosd_protocol(bin: &Path, timeout: Duration) -> Option<u32> {
    let mut command = Command::new(bin);
    command.arg("--print-protocol-version");
    let output = run_capture(command, timeout, true)?;
    output.trim().parse().ok()
}

/// Run a fixed child process with a hard timeout, killing the child on expiry.
fn run_capture(mut command: Command, timeout: Duration, require_success: bool) -> Option<String> {
    command.stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = command.spawn().ok()?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut output = String::new();
                if let Some(mut stdout) = child.stdout.take() {
                    let _ = stdout.read_to_string(&mut output);
                }
                if require_success && !status.success() {
                    return None;
                }
                return Some(output);
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

fn hostname() -> String {
    sysinfo::System::host_name().unwrap_or_else(|| "unknown".to_string())
}

fn api_build_rev() -> String {
    let rev = env!("LQOS_BUILD_REV");
    if is_hex12(rev) {
        rev.to_string()
    } else {
        "unknown".to_string()
    }
}

fn strip_source_version(version: &str) -> Option<String> {
    normalize_debian_version(version)
}

/// Normalize a Debian package version to its upstream version.
///
/// Strips an optional numeric epoch, the last-`-` Debian revision, and a
/// trailing 12-digit build stamp. A valid no-stamp version is returned as-is;
/// only a value that is not a plausible Debian upstream version is `None`.
fn normalize_debian_version(version: &str) -> Option<String> {
    let mut value = version.trim();
    if value.is_empty() {
        return None;
    }

    if let Some((epoch, rest)) = value.split_once(':') {
        if epoch.is_empty() || !epoch.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        value = rest;
    }

    if let Some((upstream, revision)) = value.rsplit_once('-') {
        if upstream.is_empty() || revision.is_empty() {
            return None;
        }
        value = upstream;
    }

    if let Some((base, suffix)) = value.rsplit_once('.')
        && suffix.len() == 12
        && suffix.bytes().all(|byte| byte.is_ascii_digit())
    {
        value = base;
    }

    if value.is_empty() || !value.as_bytes()[0].is_ascii_digit() {
        return None;
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'+' | b'~'))
    {
        return None;
    }
    Some(value.to_string())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct VersionCore(u64, u64, u64);

fn version_at_least(version: &str, minimum: VersionCore) -> bool {
    let Some(core) = parse_version_core(version) else {
        return false;
    };
    core >= minimum
}

fn parse_version_core(version: &str) -> Option<VersionCore> {
    let normalized = normalize_debian_version(version)?;
    let mut parts = normalized.split('.');
    let major = numeric_prefix(parts.next()?)?;
    let minor = numeric_prefix(parts.next()?)?;
    let patch = numeric_prefix(parts.next()?)?;
    Some(VersionCore(major, minor, patch))
}

fn numeric_prefix(value: &str) -> Option<u64> {
    let digits: String = value.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

fn is_hex12(value: &str) -> bool {
    value.len() == 12
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn rfc3339_utc(now: SystemTime) -> String {
    let seconds = now
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let days = (seconds / 86_400) as i64;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Howard Hinnant's days-from-civil inverse (proleptic Gregorian, UTC).
fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    const FIXED_NOW: u64 = 1_792_411_200; // 2026-10-19T12:00:00Z

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write script");
        let mut permissions = std::fs::metadata(&path)
            .expect("script metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).expect("chmod script");
        path
    }

    fn probe(dir: &Path, dpkg: &str, bridge: &str, lqosd: &str) -> VersionProbe {
        VersionProbe {
            dpkg_query: script(dir, "dpkg-query", dpkg),
            bridge_bin: script(dir, "bridge_rs", bridge),
            bridge_library_path: dir.to_path_buf(),
            lqosd_bin: script(dir, "lqosd", lqosd),
            timeout: Duration::from_secs(5),
            now: Arc::new(|| UNIX_EPOCH + Duration::from_secs(FIXED_NOW)),
        }
    }

    const BOTH_PACKAGES: &str = concat!(
        "printf 'libreqos\\t3.0.7.202610181425\\n';\n",
        "printf 'libreqos-dataplane\\t3.0.7.202610181425\\n'\n",
    );

    #[test]
    #[serial]
    fn full_report_maps_versions_and_build_rev() {
        let dir = tempfile::tempdir().expect("tempdir");
        let probe = probe(
            dir.path(),
            BOTH_PACKAGES,
            "printf '5f4c50065858\\n'",
            "printf '6\\n'",
        );
        let report = probe_version(&probe);
        assert_eq!(report.schema_version, 1);
        assert_eq!(report.generated_at, "2026-10-19T12:00:00Z");
        let libreqos = report.libreqos.expect("libreqos");
        assert_eq!(libreqos.version, "3.0.7.202610181425");
        assert_eq!(libreqos.source_version.as_deref(), Some("3.0.7"));
        let dataplane = report.dataplane.expect("dataplane");
        assert_eq!(dataplane.build_rev.as_deref(), Some("5f4c50065858"));
        assert_eq!(report.lqosd.expect("lqosd").protocol, 6);
        assert_eq!(report.lqos_api.version, env!("CARGO_PKG_VERSION"));
        assert!(is_hex12(&report.lqos_api.build_rev) || report.lqos_api.build_rev == "unknown");
        assert!(
            report.unavailable.is_empty()
                || report.unavailable == vec!["lqos_api.build_rev".to_string()],
            "{:?}",
            report.unavailable
        );
    }

    #[test]
    #[serial]
    fn missing_packages_are_null_and_unavailable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let probe = probe(dir.path(), "exit 1", "exit 1", "exit 1");
        let report = probe_version(&probe);
        assert!(report.libreqos.is_none());
        assert!(report.dataplane.is_none());
        assert!(report.lqosd.is_none());
        assert_eq!(
            report.unavailable,
            vec![
                "dataplane",
                "dataplane.build_rev",
                "libreqos",
                "libreqos.source_version",
                "lqosd",
            ]
        );
    }

    #[test]
    #[serial]
    fn partial_dpkg_keeps_the_libreqos_lqosd_probe() {
        let dir = tempfile::tempdir().expect("tempdir");
        let probe = probe(
            dir.path(),
            "printf 'libreqos\\t3.0.7.202610181425\\n'",
            "exit 1",
            "printf '6\\n'",
        );
        let report = probe_version(&probe);
        assert!(report.libreqos.is_some());
        assert!(report.dataplane.is_none());
        assert_eq!(report.lqosd.expect("lqosd").protocol, 6);
        assert!(report.unavailable.contains(&"dataplane".to_string()));
        assert!(
            report
                .unavailable
                .contains(&"dataplane.build_rev".to_string())
        );
        assert!(!report.unavailable.contains(&"lqosd".to_string()));
    }

    #[test]
    #[serial]
    fn source_strip_failure_is_null_and_unavailable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let probe = probe(
            dir.path(),
            "printf 'libreqos\\tnot-a-version\\n'; printf 'libreqos-dataplane\\t3.0.7.202610181425\\n'",
            "printf '5f4c50065858\\n'",
            "printf '6\\n'",
        );
        let report = probe_version(&probe);
        assert!(report.libreqos.expect("libreqos").source_version.is_none());
        assert!(
            report
                .unavailable
                .contains(&"libreqos.source_version".to_string())
        );
    }

    #[test]
    #[serial]
    fn bridge_min_gate_blocks_older_dataplane() {
        let dir = tempfile::tempdir().expect("tempdir");
        let marker = dir.path().join("bridge-ran");
        let bridge = format!("printf ran > '{}'", marker.display());
        let probe = probe(
            dir.path(),
            "printf 'libreqos\\t3.0.7.202610181425\\n'; printf 'libreqos-dataplane\\t3.0.6.202610181425\\n'",
            &bridge,
            "printf '6\\n'",
        );
        let report = probe_version(&probe);
        assert!(report.dataplane.expect("dataplane").build_rev.is_none());
        assert!(
            report
                .unavailable
                .contains(&"dataplane.build_rev".to_string())
        );
        assert!(!marker.exists(), "bridge ran below the dataplane gate");
    }

    #[test]
    #[serial]
    fn lqosd_gate_blocks_older_libreqos() {
        let dir = tempfile::tempdir().expect("tempdir");
        let marker = dir.path().join("lqosd-ran");
        let lqosd = format!("printf ran > '{}'", marker.display());
        let probe = probe(
            dir.path(),
            "printf 'libreqos\\t3.0.6.202610181425\\n'; printf 'libreqos-dataplane\\t3.0.7.202610181425\\n'",
            "printf '5f4c50065858\\n'",
            &lqosd,
        );
        let report = probe_version(&probe);
        assert!(report.lqosd.is_none());
        assert!(report.unavailable.contains(&"lqosd".to_string()));
        assert!(!marker.exists(), "lqosd ran below the libreqos gate");
    }

    #[test]
    #[serial]
    fn bridge_timeout_kills_the_child() {
        let dir = tempfile::tempdir().expect("tempdir");
        let probe = VersionProbe {
            dpkg_query: script(
                dir.path(),
                "dpkg-query",
                "printf 'libreqos\\t3.0.7.202610181425\\n'; printf 'libreqos-dataplane\\t3.0.7.202610181425\\n'",
            ),
            bridge_bin: script(dir.path(), "bridge_rs", "sleep 5"),
            bridge_library_path: dir.path().to_path_buf(),
            lqosd_bin: script(dir.path(), "lqosd", "printf '6\\n'"),
            timeout: Duration::from_millis(100),
            now: Arc::new(|| UNIX_EPOCH + Duration::from_secs(FIXED_NOW)),
        };
        let started = Instant::now();
        let report = probe_version(&probe);
        assert!(report.dataplane.expect("dataplane").build_rev.is_none());
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "timeout did not return promptly: {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    #[serial]
    async fn cache_reuses_and_expires() {
        let dir = tempfile::tempdir().expect("tempdir");
        let counter = dir.path().join("dpkg-count");
        let dpkg = format!(
            "printf x >> '{}'\nprintf 'libreqos\\t3.0.7.202610181425\\n'; printf 'libreqos-dataplane\\t3.0.7.202610181425\\n'",
            counter.display()
        );
        let probe = probe(
            dir.path(),
            &dpkg,
            "printf '5f4c50065858\\n'",
            "printf '6\\n'",
        );
        let service = VersionService::with_probe(probe, Duration::from_millis(50));
        service.report().await;
        service.report().await;
        let count = std::fs::read_to_string(&counter).expect("count");
        assert_eq!(count.len(), 1, "fresh cache should reuse the report");
        tokio::time::sleep(Duration::from_millis(80)).await;
        service.report().await;
        let count = std::fs::read_to_string(&counter).expect("count");
        assert_eq!(count.len(), 2, "expired cache should probe again");
    }

    #[tokio::test]
    #[serial]
    async fn single_flight_runs_one_probe() {
        let dir = tempfile::tempdir().expect("tempdir");
        let counter = dir.path().join("dpkg-count");
        let dpkg = format!(
            "printf x >> '{}'\nsleep 0.1\nprintf 'libreqos\\t3.0.7.202610181425\\n'; printf 'libreqos-dataplane\\t3.0.7.202610181425\\n'",
            counter.display()
        );
        let probe = probe(
            dir.path(),
            &dpkg,
            "printf '5f4c50065858\\n'",
            "printf '6\\n'",
        );
        let service = Arc::new(VersionService::with_probe(probe, Duration::from_secs(60)));
        let mut handles = Vec::new();
        for _ in 0..5 {
            let service = Arc::clone(&service);
            handles.push(tokio::spawn(async move { service.report().await }));
        }
        for handle in handles {
            handle.await.expect("report task");
        }
        let count = std::fs::read_to_string(&counter).expect("count");
        assert_eq!(count.len(), 1, "single-flight should run one probe");
    }

    #[tokio::test]
    #[serial]
    async fn slow_probe_does_not_block_the_runtime() {
        let dir = tempfile::tempdir().expect("tempdir");
        let probe = probe(
            dir.path(),
            "sleep 0.2\nprintf 'libreqos\\t3.0.7.202610181425\\n'; printf 'libreqos-dataplane\\t3.0.7.202610181425\\n'",
            "printf '5f4c50065858\\n'",
            "printf '6\\n'",
        );
        let service = Arc::new(VersionService::with_probe(probe, Duration::from_secs(60)));
        let handle = tokio::spawn(async move { service.report().await });
        let responsive = tokio::time::timeout(Duration::from_millis(50), async {
            tokio::time::sleep(Duration::from_millis(10)).await;
        })
        .await
        .is_ok();
        assert!(
            responsive,
            "the current-thread runtime was blocked by a probe"
        );
        handle.await.expect("report task");
    }

    #[test]
    fn source_version_strip_and_gate_rules() {
        assert_eq!(
            strip_source_version("3.0.7.202610181425").as_deref(),
            Some("3.0.7")
        );
        assert_eq!(strip_source_version("3.0.7").as_deref(), Some("3.0.7"));
        assert_eq!(strip_source_version("1:3.0.7-1").as_deref(), Some("3.0.7"));
        assert_eq!(
            strip_source_version("1:3.0.7.202610181425-1").as_deref(),
            Some("3.0.7")
        );
        assert_eq!(
            strip_source_version("3.0.7-beta1").as_deref(),
            Some("3.0.7")
        );
        assert_eq!(strip_source_version("not-a-version"), None);
        assert_eq!(strip_source_version("3.0.7-"), None);
        assert_eq!(strip_source_version(":3.0.7"), None);
        assert!(version_at_least("3.0.7", VersionCore(3, 0, 7)));
        assert!(version_at_least("3.0.7.202610181425", VersionCore(3, 0, 7)));
        assert!(version_at_least("3.0.7-beta1", VersionCore(3, 0, 7)));
        assert!(version_at_least("1:3.0.7-1", VersionCore(3, 0, 7)));
        assert!(!version_at_least(
            "3.0.6.202610181425",
            VersionCore(3, 0, 7)
        ));
    }

    #[test]
    #[serial]
    fn debian_epoch_and_revision_are_stripped_in_the_report() {
        let dir = tempfile::tempdir().expect("tempdir");
        let probe = probe(
            dir.path(),
            "printf 'libreqos\\t1:3.0.7-1\\n'; printf 'libreqos-dataplane\\t1:3.0.7-1\\n'",
            "printf '5f4c50065858\\n'",
            "printf '6\\n'",
        );
        let report = probe_version(&probe);
        assert_eq!(
            report.libreqos.expect("libreqos").source_version.as_deref(),
            Some("3.0.7")
        );
        assert!(
            !report
                .unavailable
                .contains(&"libreqos.source_version".to_string())
        );
        assert_eq!(
            report.dataplane.expect("dataplane").build_rev.as_deref(),
            Some("5f4c50065858")
        );
        assert_eq!(report.lqosd.expect("lqosd").protocol, 6);
    }

    #[test]
    #[serial]
    fn bridge_unknown_is_a_value_not_unavailable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let probe = probe(
            dir.path(),
            BOTH_PACKAGES,
            "printf 'unknown\\n'",
            "printf '6\\n'",
        );
        let report = probe_version(&probe);
        assert_eq!(
            report.dataplane.expect("dataplane").build_rev.as_deref(),
            Some("unknown")
        );
        assert!(
            !report
                .unavailable
                .contains(&"dataplane.build_rev".to_string())
        );
    }
}
