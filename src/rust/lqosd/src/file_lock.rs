use anyhow::Result;
use lqos_utils::process_lock::{ProcessFileLock, ProcessLockConfig};
#[cfg(test)]
use std::cell::RefCell;

const LOCK_PATH: &str = "/run/lqos/lqosd.lock";
const LOCK_DIR: &str = "/run/lqos";
const LOCK_GUARD_PATH: &str = "/run/lqos/lqosd.lock.guard";

#[cfg(test)]
thread_local! {
    static TEST_LOCK_CONFIG: RefCell<Option<ProcessLockConfig>> = const { RefCell::new(None) };
}

/// Process lock used to prevent multiple `lqosd` instances from running.
///
/// Dropping this guard removes `/run/lqos/lqosd.lock`.
#[derive(Debug)]
pub struct FileLock {
    _lock: ProcessFileLock,
}

impl FileLock {
    /// Acquires the `lqosd` process lock.
    ///
    /// A stale lock is replaced unless the recorded PID still belongs to a
    /// process whose command name contains `lqosd`.
    pub fn new() -> Result<Self> {
        let config = lock_config();
        Self::new_with_config(&config)
    }

    fn new_with_config(config: &ProcessLockConfig) -> Result<Self> {
        let lock = ProcessFileLock::acquire(config)?;
        Ok(Self { _lock: lock })
    }
}

fn lock_config() -> ProcessLockConfig {
    #[cfg(test)]
    if let Some(config) = TEST_LOCK_CONFIG.with(|config| config.borrow().clone()) {
        return config;
    }

    ProcessLockConfig::new(
        LOCK_PATH,
        LOCK_DIR,
        LOCK_GUARD_PATH,
        "start lqosd",
        "LQOSD_LOCKED",
        "lqosd",
    )
    .with_valid_process_name_contains("lqosd")
    .with_pid_only_lock_file()
    .with_strict_metadata_errors()
}

#[cfg(test)]
mod tests {
    use super::{FileLock, LOCK_DIR, LOCK_GUARD_PATH, LOCK_PATH, lock_config};
    use lqos_utils::process_lock::ProcessLockConfig;
    use std::{
        fs::{create_dir_all, read_to_string, remove_dir_all, write},
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_test_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before UNIX_EPOCH")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "libreqos-lqosd-lock-test-{}-{nanos}",
            std::process::id()
        ))
    }

    fn test_config_for(dir: &std::path::Path) -> ProcessLockConfig {
        ProcessLockConfig::new(
            dir.join("lqosd.lock"),
            dir,
            dir.join("lqosd.lock.guard"),
            "start lqosd",
            "LQOSD_LOCKED",
            "lqosd",
        )
        .with_valid_process_name_contains("lqosd")
        .with_pid_only_lock_file()
        .with_strict_metadata_errors()
    }

    struct TestLockConfigGuard;

    impl TestLockConfigGuard {
        fn set(config: ProcessLockConfig) -> Self {
            super::TEST_LOCK_CONFIG.with(|stored| {
                *stored.borrow_mut() = Some(config);
            });
            Self
        }
    }

    impl Drop for TestLockConfigGuard {
        fn drop(&mut self) {
            super::TEST_LOCK_CONFIG.with(|stored| {
                *stored.borrow_mut() = None;
            });
        }
    }

    #[test]
    fn lock_config_preserves_lqosd_contract() {
        let config = lock_config();

        assert_eq!(config.lock_path.to_string_lossy(), LOCK_PATH);
        assert_eq!(config.lock_dir.to_string_lossy(), LOCK_DIR);
        assert_eq!(config.guard_path.to_string_lossy(), LOCK_GUARD_PATH);
        assert_eq!(config.operation, "start lqosd");
        assert_eq!(config.contention_code, "LQOSD_LOCKED");
        assert_eq!(config.resource_name, "lqosd");
        assert_eq!(config.valid_process_name_contains.as_deref(), Some("lqosd"));
        assert!(config.write_pid_only);
        assert!(!config.metadata_errors_are_contention);
    }

    #[test]
    fn file_lock_new_preserves_pid_only_lock_format() {
        let dir = unique_test_dir();
        let path = dir.join("lqosd.lock");
        let _config_guard = TestLockConfigGuard::set(test_config_for(&dir));

        {
            let _lock = FileLock::new().expect("failed to create lqosd temp lock");
            let contents = read_to_string(&path).expect("failed to read lqosd temp lock");

            assert_eq!(contents, format!("{}\n", std::process::id()));
        }

        assert!(!path.exists());
        remove_dir_all(&dir).expect("failed to clean up temp test dir");
    }

    #[test]
    fn file_lock_new_preserves_strict_metadata_errors() {
        let dir = unique_test_dir();
        create_dir_all(&dir).expect("failed to create temp test dir");
        let _config_guard = TestLockConfigGuard::set(test_config_for(&dir));
        write(dir.join("lqosd.lock"), "pid=").expect("failed to write malformed temp lock");

        let error = FileLock::new().expect_err("malformed lqosd lock should remain a strict error");

        assert!(error.to_string().contains("Invalid process lock metadata"));
        remove_dir_all(&dir).expect("failed to clean up temp test dir");
    }

    #[test]
    fn file_lock_new_reports_live_lqosd_holder() {
        let dir = unique_test_dir();
        create_dir_all(&dir).expect("failed to create temp test dir");
        let _config_guard = TestLockConfigGuard::set(test_config_for(&dir));
        write(dir.join("lqosd.lock"), format!("{}\n", std::process::id()))
            .expect("failed to write live-holder temp lock");

        let error = FileLock::new().expect_err("live lqosd lock should reject a second holder");

        assert!(error.to_string().contains("LQOSD_LOCKED"));
        remove_dir_all(&dir).expect("failed to clean up temp test dir");
    }
}
