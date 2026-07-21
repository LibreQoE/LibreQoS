//! Cross-process PID-file locking helpers.

use nix::{
    errno::Errno,
    libc::{getpid, mode_t},
};
use std::{
    ffi::CString,
    fs::{File, OpenOptions, hard_link, remove_file},
    io::{ErrorKind, Read, Write},
    os::{fd::AsRawFd, unix::ffi::OsStrExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use thiserror::Error;

const STALE_LOCK_REPLACE_ATTEMPTS: usize = 3;
const LOCK_TEMP_CREATE_ATTEMPTS: usize = 8;

static TEMP_LOCK_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Error returned while acquiring or inspecting a process lock.
#[derive(Debug, Error)]
pub enum ProcessLockError {
    /// The lock file exists and appears to belong to a live process.
    #[error("{code}: {message}")]
    Contended {
        /// Stable machine-readable contention code.
        code: &'static str,
        /// Human-readable contention detail.
        message: String,
    },
    /// The lock file is malformed or missing required metadata.
    #[error("Invalid process lock metadata in {path}: {source}")]
    InvalidMetadata {
        /// Lock path whose metadata could not be parsed.
        path: PathBuf,
        /// Parsing failure source.
        source: std::num::ParseIntError,
    },
    /// The lock file does not contain a process id.
    #[error("Invalid process lock metadata in {path}: missing process id")]
    MissingPid {
        /// Lock path whose metadata did not contain a process id.
        path: PathBuf,
    },
    /// The lock file contains a process id that cannot identify one process.
    #[error("Invalid process lock metadata in {path}: process id must be positive, got {pid}")]
    InvalidPid {
        /// Lock path whose metadata contained the invalid process id.
        path: PathBuf,
        /// Invalid process id value.
        pid: i32,
    },
    /// A filesystem or operating-system call failed.
    #[error("{context}: {source}")]
    Io {
        /// Operation being attempted when the error occurred.
        context: &'static str,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// A path could not be represented as a C string for libc calls.
    #[error("Path contains an interior NUL byte: {path}")]
    InvalidPathCString {
        /// Path that could not be converted.
        path: PathBuf,
    },
    /// The lock could not be acquired after replacing stale metadata.
    #[error("{message}")]
    AcquisitionFailed {
        /// Human-readable acquisition failure detail.
        message: String,
    },
}

impl ProcessLockError {
    fn io(context: &'static str, source: std::io::Error) -> Self {
        Self::Io { context, source }
    }
}

/// Configuration for acquiring a [`ProcessFileLock`].
#[derive(Clone, Debug)]
pub struct ProcessLockConfig {
    /// Lock file path.
    pub lock_path: PathBuf,
    /// Directory that contains the lock file.
    pub lock_dir: PathBuf,
    /// Advisory guard file used to serialize stale-lock replacement.
    pub guard_path: PathBuf,
    /// Diagnostic operation recorded in the lock file.
    pub operation: String,
    /// Process names accepted as active holders of this lock.
    pub valid_process_name_contains: Option<String>,
    /// Whether new lock files should use the legacy PID-only format.
    pub write_pid_only: bool,
    /// Whether unreadable or malformed metadata should be reported as lock
    /// contention.
    pub metadata_errors_are_contention: bool,
    /// Stable machine-readable contention code.
    pub contention_code: &'static str,
    /// Human-readable name for the locked resource.
    pub resource_name: String,
}

impl ProcessLockConfig {
    /// Creates lock configuration for a lock file under `lock_dir`.
    pub fn new(
        lock_path: impl Into<PathBuf>,
        lock_dir: impl Into<PathBuf>,
        guard_path: impl Into<PathBuf>,
        operation: impl Into<String>,
        contention_code: &'static str,
        resource_name: impl Into<String>,
    ) -> Self {
        Self {
            lock_path: lock_path.into(),
            lock_dir: lock_dir.into(),
            guard_path: guard_path.into(),
            operation: operation.into(),
            valid_process_name_contains: None,
            write_pid_only: false,
            metadata_errors_are_contention: true,
            contention_code,
            resource_name: resource_name.into(),
        }
    }

    /// Restricts live-lock detection to processes whose command name contains
    /// `needle`.
    pub fn with_valid_process_name_contains(mut self, needle: impl Into<String>) -> Self {
        self.valid_process_name_contains = Some(needle.into());
        self
    }

    /// Writes new lock files as a bare PID for compatibility with existing
    /// tooling.
    pub fn with_pid_only_lock_file(mut self) -> Self {
        self.write_pid_only = true;
        self
    }

    /// Returns malformed metadata as the original acquisition error instead of
    /// converting it to contention.
    pub fn with_strict_metadata_errors(mut self) -> Self {
        self.metadata_errors_are_contention = false;
        self
    }
}

/// Cross-process lock represented by an on-disk PID file.
///
/// The lock writes structured metadata atomically through a temporary file and
/// hard link. Dropping the guard removes the lock file. Acquisition uses a
/// secondary `flock` guard so stale-lock replacement is serialized between
/// competing processes.
#[derive(Debug)]
pub struct ProcessFileLock {
    lock_path: PathBuf,
}

#[derive(Debug)]
struct AcquisitionGuard {
    file: File,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LockMetadata {
    pid: i32,
    process: Option<String>,
    operation: Option<String>,
    created_unix: Option<u64>,
    process_start_ticks: Option<u64>,
}

impl LockMetadata {
    fn current(operation: &str) -> Self {
        Self {
            pid: unsafe { getpid() },
            process: current_process_name(),
            operation: Some(sanitize_lock_field(operation)),
            created_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|duration| duration.as_secs()),
            process_start_ticks: process_start_ticks_for_pid(unsafe { getpid() }),
        }
    }

    fn parse(contents: &str, path: &Path) -> Result<Self, ProcessLockError> {
        let trimmed = contents.trim();
        if let Ok(pid) = trimmed.parse::<i32>() {
            validate_pid(pid, path)?;
            return Ok(Self {
                pid,
                process: None,
                operation: None,
                created_unix: None,
                process_start_ticks: None,
            });
        }

        let mut pid = None;
        let mut process = None;
        let mut operation = None;
        let mut created_unix = None;
        let mut process_start_ticks = None;
        for line in contents.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "pid" => {
                    let parsed = value.trim().parse::<i32>().map_err(|source| {
                        ProcessLockError::InvalidMetadata {
                            path: path.to_path_buf(),
                            source,
                        }
                    })?;
                    validate_pid(parsed, path)?;
                    pid = Some(parsed);
                }
                "process" => process = Some(value.trim().to_string()),
                "operation" => {
                    operation = Some(value.trim().to_string());
                }
                "created_unix" => {
                    created_unix = Some(value.trim().parse::<u64>().map_err(|source| {
                        ProcessLockError::InvalidMetadata {
                            path: path.to_path_buf(),
                            source,
                        }
                    })?)
                }
                "process_start_ticks" => {
                    process_start_ticks = Some(value.trim().parse::<u64>().map_err(|source| {
                        ProcessLockError::InvalidMetadata {
                            path: path.to_path_buf(),
                            source,
                        }
                    })?)
                }
                _ => {}
            }
        }

        let Some(pid) = pid else {
            return Err(ProcessLockError::MissingPid {
                path: path.to_path_buf(),
            });
        };

        Ok(Self {
            pid,
            process,
            operation,
            created_unix,
            process_start_ticks,
        })
    }

    fn serialize(&self) -> String {
        let process = self.process.as_deref().unwrap_or("unknown");
        let operation = self.operation.as_deref().unwrap_or("unknown");
        let created_unix = self
            .created_unix
            .map(|seconds| seconds.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let mut serialized = format!(
            "pid={}\nprocess={}\noperation={}\ncreated_unix={}\n",
            self.pid, process, operation, created_unix
        );
        if let Some(process_start_ticks) = self.process_start_ticks {
            serialized.push_str(&format!("process_start_ticks={process_start_ticks}\n"));
        }
        serialized
    }

    fn describe_holder(&self) -> String {
        let mut parts = vec![format!("pid={}", self.pid)];
        if let Some(process) = self.process.as_deref().filter(|value| !value.is_empty()) {
            parts.push(format!("process={process}"));
        }
        if let Some(operation) = self.operation.as_deref().filter(|value| !value.is_empty()) {
            parts.push(format!("operation={operation}"));
        }
        if let Some(created_unix) = self.created_unix {
            parts.push(format!("created_unix={created_unix}"));
        }
        if let Some(process_start_ticks) = self.process_start_ticks {
            parts.push(format!("process_start_ticks={process_start_ticks}"));
        }
        parts.join(", ")
    }
}

impl ProcessFileLock {
    /// Acquires a process lock using `config`.
    ///
    /// This function creates the lock directory when missing, writes the current
    /// process metadata to the lock file, and removes stale lock files whose PID
    /// no longer belongs to a valid holder.
    pub fn acquire(config: &ProcessLockConfig) -> Result<Self, ProcessLockError> {
        Self::check_directory(&config.lock_dir)?;
        let _guard = Self::acquire_guard(&config.guard_path)?;
        for _ in 0..STALE_LOCK_REPLACE_ATTEMPTS {
            if Self::create_lock(
                &config.lock_path,
                &config.lock_dir,
                &config.operation,
                config.write_pid_only,
            )? {
                return Ok(Self {
                    lock_path: config.lock_path.clone(),
                });
            }

            let metadata = match Self::read_lock_metadata(&config.lock_path) {
                Ok(metadata) => metadata,
                Err(ProcessLockError::Io { source, .. })
                    if source.kind() == ErrorKind::NotFound =>
                {
                    continue;
                }
                Err(ProcessLockError::InvalidPid { .. }) => {
                    Self::remove_stale_lock(&config.lock_path)?;
                    continue;
                }
                Err(err) if !config.metadata_errors_are_contention => return Err(err),
                Err(err) => {
                    return Err(ProcessLockError::Contended {
                        code: config.contention_code,
                        message: format!(
                            "{} is locked by another process (lock metadata unreadable: {err}).",
                            config.resource_name
                        ),
                    });
                }
            };
            if Self::is_lock_valid(&metadata, config.valid_process_name_contains.as_deref()) {
                return Err(ProcessLockError::Contended {
                    code: config.contention_code,
                    message: format!(
                        "{} is locked by another process ({}).",
                        config.resource_name,
                        metadata.describe_holder()
                    ),
                });
            }

            Self::remove_stale_lock(&config.lock_path)?;
        }

        Err(ProcessLockError::AcquisitionFailed {
            message: format!(
                "Unable to acquire {} after replacing a stale lock.",
                config.resource_name
            ),
        })
    }

    fn acquire_guard(guard_path: &Path) -> Result<AcquisitionGuard, ProcessLockError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(guard_path)
            .map_err(|err| ProcessLockError::io("open process lock guard", err))?;
        let _ = chmod_path(guard_path, 0o666);
        let ret = unsafe { nix::libc::flock(file.as_raw_fd(), nix::libc::LOCK_EX) };
        if ret == 0 {
            Ok(AcquisitionGuard { file })
        } else {
            Err(ProcessLockError::io(
                "acquire process lock guard",
                std::io::Error::from_raw_os_error(Errno::last_raw()),
            ))
        }
    }

    fn remove_stale_lock(lock_path: &Path) -> Result<(), ProcessLockError> {
        match remove_file(lock_path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
            Err(err) => Err(ProcessLockError::io("remove stale process lock", err)),
        }
    }

    fn read_lock_metadata(lock_path: &Path) -> Result<LockMetadata, ProcessLockError> {
        let mut f = File::open(lock_path)
            .map_err(|err| ProcessLockError::io("open process lock metadata", err))?;
        let mut contents = String::new();
        f.read_to_string(&mut contents)
            .map_err(|err| ProcessLockError::io("read process lock metadata", err))?;
        LockMetadata::parse(&contents, lock_path)
    }

    fn is_lock_valid(metadata: &LockMetadata, process_name_contains: Option<&str>) -> bool {
        let ret = unsafe { nix::libc::kill(metadata.pid, 0) };
        if ret != 0 {
            return Errno::last() != Errno::ESRCH;
        }

        if let Some(expected_start_ticks) = metadata.process_start_ticks {
            let Some(actual_start_ticks) = process_start_ticks_for_pid(metadata.pid) else {
                return true;
            };
            if actual_start_ticks != expected_start_ticks {
                return false;
            }
        }

        let Some(needle) = process_name_contains else {
            return true;
        };
        process_name_matches_pid(metadata.pid, needle)
    }

    fn create_lock(
        lock_path: &Path,
        lock_dir: &Path,
        operation: &str,
        write_pid_only: bool,
    ) -> Result<bool, ProcessLockError> {
        let (mut f, temp_path) = Self::create_temp_lock_file(lock_dir)?;
        let serialized = if write_pid_only {
            format!("{}\n", unsafe { getpid() })
        } else {
            LockMetadata::current(operation).serialize()
        };
        if let Err(err) = f.write_all(serialized.as_bytes()) {
            let _ = remove_file(&temp_path);
            return Err(ProcessLockError::io("write process lock metadata", err));
        }
        if let Err(err) = f.sync_all() {
            let _ = remove_file(&temp_path);
            return Err(ProcessLockError::io("sync process lock metadata", err));
        }
        drop(f);
        let link_result = hard_link(&temp_path, lock_path);
        let _ = remove_file(&temp_path);
        match link_result {
            Ok(()) => {}
            Err(err) if err.kind() == ErrorKind::AlreadyExists => return Ok(false),
            Err(err) => return Err(ProcessLockError::io("link process lock", err)),
        }
        let _ = chmod_path(lock_path, 0o666);
        Ok(true)
    }

    fn create_temp_lock_file(lock_dir: &Path) -> Result<(File, PathBuf), ProcessLockError> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        for _ in 0..LOCK_TEMP_CREATE_ATTEMPTS {
            let sequence = TEMP_LOCK_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let temp_path = lock_dir.join(format!(
                ".libreqos-process-lock.{}.{}.{}.tmp",
                std::process::id(),
                nanos,
                sequence
            ));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)
            {
                Ok(file) => return Ok((file, temp_path)),
                Err(err) if err.kind() == ErrorKind::AlreadyExists => continue,
                Err(err) => {
                    return Err(ProcessLockError::io(
                        "create temporary process lock file",
                        err,
                    ));
                }
            }
        }

        Err(ProcessLockError::AcquisitionFailed {
            message: format!(
                "Unable to create a unique temporary process lock file after {LOCK_TEMP_CREATE_ATTEMPTS} attempts."
            ),
        })
    }

    fn check_directory(lock_dir: &Path) -> Result<(), ProcessLockError> {
        if lock_dir.exists() && lock_dir.is_dir() {
            Ok(())
        } else {
            std::fs::create_dir(lock_dir)
                .map_err(|err| ProcessLockError::io("create process lock directory", err))?;
            let _ = chmod_path(lock_dir, 0o777);
            Ok(())
        }
    }
}

impl Drop for ProcessFileLock {
    fn drop(&mut self) {
        let _ = remove_file(&self.lock_path);
    }
}

impl Drop for AcquisitionGuard {
    fn drop(&mut self) {
        let _ = unsafe { nix::libc::flock(self.file.as_raw_fd(), nix::libc::LOCK_UN) };
    }
}

fn chmod_path(path: &Path, mode: mode_t) -> Result<(), ProcessLockError> {
    let unix_path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        ProcessLockError::InvalidPathCString {
            path: path.to_path_buf(),
        }
    })?;
    let ret = unsafe { nix::libc::chmod(unix_path.as_ptr(), mode) };
    if ret == 0 {
        Ok(())
    } else {
        Err(ProcessLockError::io(
            "chmod process lock path",
            std::io::Error::from_raw_os_error(Errno::last_raw()),
        ))
    }
}

fn current_process_name() -> Option<String> {
    std::fs::read_to_string("/proc/self/comm")
        .ok()
        .and_then(|name| non_empty_sanitized(name.trim()))
        .or_else(|| {
            std::env::args()
                .next()
                .and_then(|arg| non_empty_sanitized(&arg))
        })
}

fn process_name_matches_pid(pid: i32, needle: &str) -> bool {
    process_name_matches(
        process_argv0_for_pid(pid).as_deref(),
        process_comm_for_pid(pid).as_deref(),
        needle,
    )
}

fn process_name_matches(argv0: Option<&str>, comm: Option<&str>, needle: &str) -> bool {
    argv0.is_some_and(|name| argv0_name_matches(name, needle))
        || comm.is_some_and(|name| name.contains(needle))
}

fn argv0_name_matches(argv0: &str, needle: &str) -> bool {
    Path::new(argv0)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.contains(needle))
}

fn process_argv0_for_pid(pid: i32) -> Option<String> {
    std::fs::read(format!("/proc/{pid}/cmdline"))
        .ok()
        .and_then(|raw| process_argv0_from_cmdline_bytes(&raw))
}

fn process_comm_for_pid(pid: i32) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .ok()
        .and_then(|name| non_empty_sanitized(name.trim()))
}

fn process_start_ticks_for_pid(pid: i32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, fields) = stat.rsplit_once(')')?;
    fields.split_whitespace().nth(19)?.parse().ok()
}

fn process_argv0_from_cmdline_bytes(raw: &[u8]) -> Option<String> {
    raw.split(|byte| *byte == 0)
        .next()
        .filter(|arg| !arg.is_empty())
        .map(|arg| String::from_utf8_lossy(arg).to_string())
        .and_then(|arg0| non_empty_sanitized(&arg0))
}

fn validate_pid(pid: i32, path: &Path) -> Result<(), ProcessLockError> {
    if pid > 0 {
        Ok(())
    } else {
        Err(ProcessLockError::InvalidPid {
            path: path.to_path_buf(),
            pid,
        })
    }
}

fn non_empty_sanitized(value: &str) -> Option<String> {
    let trimmed = sanitize_lock_field(value);
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn sanitize_lock_field(value: &str) -> String {
    value
        .chars()
        .filter(|character| *character != '\n' && *character != '\r')
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        LockMetadata, ProcessFileLock, ProcessLockConfig, ProcessLockError,
        process_argv0_from_cmdline_bytes, process_name_matches, process_start_ticks_for_pid,
    };
    use std::{
        fs::{create_dir_all, read_to_string, remove_dir_all, write},
        os::unix::ffi::OsStringExt,
        path::PathBuf,
        sync::mpsc,
        thread,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_test_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before UNIX_EPOCH")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "libreqos-process-lock-test-{}-{nanos}",
            std::process::id()
        ))
    }

    fn config_for(dir: &std::path::Path) -> ProcessLockConfig {
        ProcessLockConfig::new(
            dir.join("test.lock"),
            dir,
            dir.join("test.lock.guard"),
            "load effective overrides",
            "TEST_LOCKED",
            "Test resource",
        )
    }

    fn pid_only_config_for(dir: &std::path::Path) -> ProcessLockConfig {
        config_for(dir).with_pid_only_lock_file()
    }

    fn unique_non_utf8_test_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before UNIX_EPOCH")
            .as_nanos();
        let mut bytes =
            format!("libreqos-process-lock-test-{}-{nanos}-", std::process::id()).into_bytes();
        bytes.push(0xff);
        std::env::temp_dir().join(std::ffi::OsString::from_vec(bytes))
    }

    #[test]
    fn parses_legacy_pid_only_lock_file() {
        let metadata = LockMetadata::parse("12345\n", std::path::Path::new("test.lock"))
            .expect("legacy pid lock should parse");

        assert_eq!(metadata.pid, 12345);
        assert_eq!(metadata.process, None);
        assert_eq!(metadata.operation, None);
        assert_eq!(metadata.created_unix, None);
    }

    #[test]
    fn rejects_nonpositive_legacy_pid() {
        let error = LockMetadata::parse("0\n", std::path::Path::new("test.lock"))
            .expect_err("PID zero should be rejected");

        assert!(matches!(error, ProcessLockError::InvalidPid { pid: 0, .. }));
    }

    #[test]
    fn rejects_nonpositive_structured_pid() {
        let error = LockMetadata::parse(
            "pid=-1\nprocess=old-holder\noperation=save overrides\ncreated_unix=1800000000\n",
            std::path::Path::new("test.lock"),
        )
        .expect_err("negative PID should be rejected");

        assert!(matches!(
            error,
            ProcessLockError::InvalidPid { pid: -1, .. }
        ));
    }

    #[test]
    fn parses_metadata_lock_file() {
        let metadata = LockMetadata::parse(
            "pid=12345\nprocess=lqosd\noperation=load effective overrides\ncreated_unix=1800000000\n",
            std::path::Path::new("test.lock"),
        )
        .expect("metadata lock should parse");

        assert_eq!(metadata.pid, 12345);
        assert_eq!(metadata.process.as_deref(), Some("lqosd"));
        assert_eq!(
            metadata.operation.as_deref(),
            Some("load effective overrides")
        );
        assert_eq!(metadata.created_unix, Some(1_800_000_000));
        assert_eq!(metadata.process_start_ticks, None);
    }

    #[test]
    fn holder_description_includes_available_metadata() {
        let metadata = LockMetadata::parse(
            "pid=12345\nprocess=node_manager\noperation=save overrides\ncreated_unix=1800000000\n",
            std::path::Path::new("test.lock"),
        )
        .expect("metadata lock should parse");

        assert_eq!(
            metadata.describe_holder(),
            "pid=12345, process=node_manager, operation=save overrides, created_unix=1800000000"
        );
    }

    #[test]
    fn lock_file_records_operation_metadata_and_cleans_up_on_drop() {
        let dir = unique_test_dir();
        create_dir_all(&dir).expect("failed to create temp test dir");
        let path = dir.join("test.lock");

        {
            let _lock = ProcessFileLock::acquire(&config_for(&dir))
                .expect("failed to create lock in temp dir");
            let contents = read_to_string(&path).expect("failed to read temp lock");

            assert!(contents.contains(&format!("pid={}", std::process::id())));
            assert!(contents.contains("process="));
            assert!(contents.contains("operation=load effective overrides"));
            assert!(contents.contains("created_unix="));
            assert!(contents.contains("process_start_ticks="));
        }

        assert!(!path.exists());
        remove_dir_all(&dir).expect("failed to clean up temp test dir");
    }

    #[test]
    fn start_ticks_distinguish_reused_pids() {
        let mut metadata = LockMetadata::current("save operator overrides");
        let current_start_ticks = process_start_ticks_for_pid(metadata.pid)
            .expect("current process should expose Linux start ticks");
        assert!(ProcessFileLock::is_lock_valid(&metadata, None));

        metadata.process_start_ticks = Some(current_start_ticks.saturating_add(1));
        assert!(!ProcessFileLock::is_lock_valid(&metadata, None));
    }

    #[test]
    fn metadata_without_start_ticks_round_trips() {
        let metadata = LockMetadata {
            pid: 12345,
            process: Some("lqos_overrides".to_string()),
            operation: Some("save operator overrides".to_string()),
            created_unix: Some(1_800_000_000),
            process_start_ticks: None,
        };
        let parsed = LockMetadata::parse(&metadata.serialize(), std::path::Path::new("test.lock"))
            .expect("metadata without start ticks should remain readable");

        assert_eq!(parsed, metadata);
    }

    #[test]
    fn pid_only_lock_file_preserves_legacy_format() {
        let dir = unique_test_dir();
        create_dir_all(&dir).expect("failed to create temp test dir");
        let path = dir.join("test.lock");

        {
            let _lock = ProcessFileLock::acquire(&pid_only_config_for(&dir))
                .expect("failed to create PID-only lock in temp dir");
            let contents = read_to_string(&path).expect("failed to read temp lock");

            assert_eq!(contents, format!("{}\n", std::process::id()));
        }

        assert!(!path.exists());
        remove_dir_all(&dir).expect("failed to clean up temp test dir");
    }

    #[test]
    fn lock_acquire_supports_non_utf8_paths() {
        let dir = unique_non_utf8_test_dir();
        let path = dir.join("test.lock");

        {
            let _lock = ProcessFileLock::acquire(&config_for(&dir))
                .expect("failed to create lock under non-UTF8 path");
            let contents = read_to_string(&path).expect("failed to read non-UTF8 temp lock");

            assert!(contents.contains(&format!("pid={}", std::process::id())));
            assert!(contents.contains("operation=load effective overrides"));
        }

        assert!(!path.exists());
        remove_dir_all(&dir).expect("failed to clean up temp test dir");
    }

    #[test]
    fn live_lock_error_reports_existing_holder_metadata() {
        let dir = unique_test_dir();
        create_dir_all(&dir).expect("failed to create temp test dir");
        let path = dir.join("test.lock");
        write(
            &path,
            format!(
                "pid={}\nprocess=test-holder\noperation=save overrides\ncreated_unix=1800000000\n",
                std::process::id()
            ),
        )
        .expect("failed to write temp lock");

        let error = ProcessFileLock::acquire(&config_for(&dir))
            .expect_err("live lock should reject a second holder");
        let ProcessLockError::Contended { code, message } = error else {
            panic!("expected contention error");
        };

        assert_eq!(code, "TEST_LOCKED");
        assert!(message.contains("pid="));
        assert!(message.contains("process=test-holder"));
        assert!(message.contains("operation=save overrides"));
        assert!(message.contains("created_unix=1800000000"));

        remove_dir_all(&dir).expect("failed to clean up temp test dir");
    }

    #[test]
    fn malformed_lock_file_is_reported_as_lock_contention() {
        let dir = unique_test_dir();
        create_dir_all(&dir).expect("failed to create temp test dir");
        let path = dir.join("test.lock");
        write(&path, "pid=").expect("failed to write malformed temp lock");

        let error = ProcessFileLock::acquire(&config_for(&dir))
            .expect_err("malformed lock metadata should be reported as contention");
        let message = error.to_string();

        assert!(message.contains("locked by another process"));
        assert!(message.contains("lock metadata unreadable"));

        remove_dir_all(&dir).expect("failed to clean up temp test dir");
    }

    #[test]
    fn invalid_pid_file_is_treated_as_stale() {
        let dir = unique_test_dir();
        create_dir_all(&dir).expect("failed to create temp test dir");
        let path = dir.join("test.lock");
        write(&path, "0\n").expect("failed to write invalid PID temp lock");

        {
            let _lock = ProcessFileLock::acquire(&config_for(&dir))
                .expect("invalid PID metadata should be treated as stale");
            let contents = read_to_string(&path).expect("failed to read temp lock");

            assert!(contents.contains(&format!("pid={}", std::process::id())));
            assert!(contents.contains("operation=load effective overrides"));
        }

        remove_dir_all(&dir).expect("failed to clean up temp test dir");
    }

    #[test]
    fn strict_metadata_errors_preserve_parse_failure() {
        let dir = unique_test_dir();
        create_dir_all(&dir).expect("failed to create temp test dir");
        let path = dir.join("test.lock");
        write(&path, "pid=").expect("failed to write malformed temp lock");

        let error = ProcessFileLock::acquire(&config_for(&dir).with_strict_metadata_errors())
            .expect_err("strict metadata errors should not be reported as contention");

        assert!(matches!(error, ProcessLockError::InvalidMetadata { .. }));

        remove_dir_all(&dir).expect("failed to clean up temp test dir");
    }

    #[test]
    fn strict_metadata_errors_preserve_created_unix_parse_failure() {
        let dir = unique_test_dir();
        create_dir_all(&dir).expect("failed to create temp test dir");
        let path = dir.join("test.lock");
        write(
            &path,
            "pid=12345\nprocess=test-holder\noperation=save overrides\ncreated_unix=not-a-number\n",
        )
        .expect("failed to write malformed created_unix temp lock");

        let error = ProcessFileLock::acquire(&config_for(&dir).with_strict_metadata_errors())
            .expect_err("strict metadata errors should preserve created_unix parse failures");

        assert!(matches!(error, ProcessLockError::InvalidMetadata { .. }));

        remove_dir_all(&dir).expect("failed to clean up temp test dir");
    }

    #[test]
    fn strict_metadata_errors_preserve_missing_pid_failure() {
        let dir = unique_test_dir();
        create_dir_all(&dir).expect("failed to create temp test dir");
        let path = dir.join("test.lock");
        write(
            &path,
            "process=test-holder\noperation=save overrides\ncreated_unix=1800000000\n",
        )
        .expect("failed to write truncated temp lock");

        let error = ProcessFileLock::acquire(&config_for(&dir).with_strict_metadata_errors())
            .expect_err("strict metadata errors should preserve missing PID");

        assert!(matches!(error, ProcessLockError::MissingPid { .. }));

        remove_dir_all(&dir).expect("failed to clean up temp test dir");
    }

    #[test]
    fn strict_metadata_errors_still_replace_invalid_pid() {
        let dir = unique_test_dir();
        create_dir_all(&dir).expect("failed to create temp test dir");
        let path = dir.join("test.lock");
        write(&path, "-1\n").expect("failed to write invalid PID temp lock");

        {
            let _lock = ProcessFileLock::acquire(&config_for(&dir).with_strict_metadata_errors())
                .expect("strict metadata should still replace invalid PID");
            let contents = read_to_string(&path).expect("failed to read temp lock");

            assert!(contents.contains(&format!("pid={}", std::process::id())));
            assert!(contents.contains("operation=load effective overrides"));
        }

        remove_dir_all(&dir).expect("failed to clean up temp test dir");
    }

    #[test]
    fn truncated_metadata_lock_missing_pid_is_reported_as_contention() {
        let dir = unique_test_dir();
        create_dir_all(&dir).expect("failed to create temp test dir");
        let path = dir.join("test.lock");
        write(
            &path,
            "process=test-holder\noperation=save overrides\ncreated_unix=1800000000\n",
        )
        .expect("failed to write truncated temp lock");

        let error = ProcessFileLock::acquire(&config_for(&dir))
            .expect_err("truncated lock metadata should be reported as contention");
        let message = error.to_string();

        assert!(message.contains("locked by another process"));
        assert!(message.contains("lock metadata unreadable"));

        remove_dir_all(&dir).expect("failed to clean up temp test dir");
    }

    #[test]
    fn stale_legacy_lock_is_replaced_with_metadata_lock() {
        let dir = unique_test_dir();
        create_dir_all(&dir).expect("failed to create temp test dir");
        let path = dir.join("test.lock");
        write(&path, "2147483647\n").expect("failed to write stale temp lock");

        {
            let _lock = ProcessFileLock::acquire(&config_for(&dir))
                .expect("stale legacy lock should be replaceable");
            let contents = read_to_string(&path).expect("failed to read temp lock");

            assert!(contents.contains(&format!("pid={}", std::process::id())));
            assert!(contents.contains("operation=load effective overrides"));
        }

        remove_dir_all(&dir).expect("failed to clean up temp test dir");
    }

    #[test]
    fn stale_structured_lock_is_replaced_with_metadata_lock() {
        let dir = unique_test_dir();
        create_dir_all(&dir).expect("failed to create temp test dir");
        let path = dir.join("test.lock");
        write(
            &path,
            "pid=2147483647\nprocess=old-holder\noperation=save overrides\ncreated_unix=1800000000\n",
        )
        .expect("failed to write stale structured temp lock");

        {
            let _lock = ProcessFileLock::acquire(&config_for(&dir))
                .expect("stale structured lock should be replaceable");
            let contents = read_to_string(&path).expect("failed to read temp lock");

            assert!(contents.contains(&format!("pid={}", std::process::id())));
            assert!(contents.contains("operation=load effective overrides"));
        }

        remove_dir_all(&dir).expect("failed to clean up temp test dir");
    }

    #[test]
    fn guard_blocks_other_acquirers() {
        let dir = unique_test_dir();
        create_dir_all(&dir).expect("failed to create temp test dir");
        let guard_path = dir.join("test.lock.guard");
        let primary_guard =
            ProcessFileLock::acquire_guard(&guard_path).expect("failed to acquire primary guard");
        let (sender, receiver) = mpsc::channel();
        thread::scope(|scope| {
            let sender = sender.clone();
            let guard_path = guard_path.clone();
            let handle = scope.spawn(move || {
                let _blocked_guard = ProcessFileLock::acquire_guard(&guard_path)
                    .expect("failed to acquire secondary guard");
                sender.send(()).expect("failed to send guard status");
            });
            assert!(receiver.try_recv().is_err());
            drop(primary_guard);
            receiver.recv().expect("secondary guard did not unblock");
            handle.join().expect("secondary guard thread panicked");
        });

        remove_dir_all(&dir).expect("failed to clean up temp test dir");
    }

    #[test]
    fn process_name_filter_rejects_unrelated_live_holder_as_stale() {
        let dir = unique_test_dir();
        create_dir_all(&dir).expect("failed to create temp test dir");
        let path = dir.join("test.lock");
        write(&path, format!("{}\n", std::process::id())).expect("failed to write temp lock");

        {
            let config = config_for(&dir)
                .with_valid_process_name_contains("name-that-should-not-match-this-test-process");
            let _lock = ProcessFileLock::acquire(&config)
                .expect("unmatched live process name should be treated as stale");
        }

        remove_dir_all(&dir).expect("failed to clean up temp test dir");
    }

    #[test]
    fn process_name_filter_accepts_current_process_lookup() {
        let dir = unique_test_dir();
        create_dir_all(&dir).expect("failed to create temp test dir");
        let path = dir.join("test.lock");
        write(&path, format!("{}\n", std::process::id())).expect("failed to write temp lock");

        let current_exe = std::env::current_exe().expect("failed to read current executable path");
        let process_name = current_exe
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.split('-').next())
            .expect("test executable should have a UTF-8 name");
        let config = config_for(&dir).with_valid_process_name_contains(process_name);
        let error = ProcessFileLock::acquire(&config)
            .expect_err("matched live process name should be treated as contention");
        assert!(matches!(error, ProcessLockError::Contended { .. }));

        remove_dir_all(&dir).expect("failed to clean up temp test dir");
    }

    #[test]
    fn process_name_lookup_uses_only_cmdline_argv0() {
        let process_name = process_argv0_from_cmdline_bytes(
            b"/opt/libreqos/bin/lqosd\0--label\0unrelated-lqosd-marker\0",
        )
        .expect("cmdline should expose argv0");

        assert_eq!(process_name, "/opt/libreqos/bin/lqosd");
    }

    #[test]
    fn process_name_lookup_rejects_marker_after_empty_argv0() {
        assert_eq!(
            process_argv0_from_cmdline_bytes(b"\0--label\0unrelated-lqosd-marker\0"),
            None
        );
    }

    #[test]
    fn process_name_match_accepts_argv0_or_comm() {
        assert!(process_name_matches(
            Some("/opt/libreqos/bin/lqosd"),
            Some("generic-wrapper"),
            "lqosd"
        ));
        assert!(process_name_matches(
            Some("generic-wrapper"),
            Some("lqosd"),
            "lqosd"
        ));
        assert!(!process_name_matches(
            Some("generic-wrapper"),
            Some("unrelated"),
            "lqosd"
        ));
    }

    #[test]
    fn process_name_match_ignores_argv0_directory_names() {
        assert!(!process_name_matches(
            Some("/tmp/lqosd-wrapper/custom-daemon"),
            Some("unrelated"),
            "lqosd"
        ));
    }
}
