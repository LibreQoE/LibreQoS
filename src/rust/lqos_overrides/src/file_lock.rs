use anyhow::Result;
use lqos_utils::process_lock::{ProcessFileLock, ProcessLockConfig};
#[cfg(test)]
use std::cell::RefCell;

const LOCK_PATH: &str = "/run/lqos/lqos_overrides.lock";
const LOCK_DIR: &str = "/run/lqos";
const LOCK_GUARD_PATH: &str = "/run/lqos/lqos_overrides.lock.guard";
const LOCK_CONTENTION_CODE: &str = "LQOS_OVERRIDES_LOCKED";

#[cfg(test)]
thread_local! {
    static TEST_LOCK_CONFIG: RefCell<Option<ProcessLockConfig>> = const { RefCell::new(None) };
}

/// Cross-process lock used while mutating operator-owned override files.
///
/// Dropping this guard removes `/run/lqos/lqos_overrides.lock`.
#[derive(Debug)]
pub struct FileLock {
    _lock: ProcessFileLock,
}

impl FileLock {
    /// Acquires the lock and records the caller operation for diagnostics.
    pub fn new_for_operation(operation: &str) -> Result<Self> {
        let config = lock_config(operation);
        Self::new_with_config(&config)
    }

    fn new_with_config(config: &ProcessLockConfig) -> Result<Self> {
        let lock = ProcessFileLock::acquire(config)?;
        Ok(Self { _lock: lock })
    }
}

fn lock_config(operation: &str) -> ProcessLockConfig {
    #[cfg(test)]
    if let Some(config) = TEST_LOCK_CONFIG.with(|config| config.borrow().clone()) {
        return config;
    }

    ProcessLockConfig::new(
        LOCK_PATH,
        LOCK_DIR,
        LOCK_GUARD_PATH,
        operation,
        LOCK_CONTENTION_CODE,
        "The LibreQoS overrides file lock",
    )
}

#[cfg(test)]
mod tests {
    use super::{
        FileLock, LOCK_CONTENTION_CODE, LOCK_DIR, LOCK_GUARD_PATH, LOCK_PATH, lock_config,
    };
    use lqos_utils::process_lock::ProcessLockConfig;
    use std::{
        fs::{read_to_string, remove_dir_all},
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_test_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before UNIX_EPOCH")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "libreqos-overrides-lock-test-{}-{nanos}",
            std::process::id()
        ))
    }

    fn test_config_for(dir: &std::path::Path, operation: &str) -> ProcessLockConfig {
        ProcessLockConfig::new(
            dir.join("lqos_overrides.lock"),
            dir,
            dir.join("lqos_overrides.lock.guard"),
            operation,
            LOCK_CONTENTION_CODE,
            "The LibreQoS overrides file lock",
        )
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
    fn lock_config_preserves_overrides_contract() {
        let config = lock_config("save operator overrides");

        assert_eq!(config.lock_path.to_string_lossy(), LOCK_PATH);
        assert_eq!(config.lock_dir.to_string_lossy(), LOCK_DIR);
        assert_eq!(config.guard_path.to_string_lossy(), LOCK_GUARD_PATH);
        assert_eq!(config.operation, "save operator overrides");
        assert_eq!(config.contention_code, LOCK_CONTENTION_CODE);
        assert_eq!(config.resource_name, "The LibreQoS overrides file lock");
        assert_eq!(config.valid_process_name_contains, None);
        assert!(!config.write_pid_only);
        assert!(config.metadata_errors_are_contention);
    }

    #[test]
    fn file_lock_new_records_structured_operation_metadata() {
        let dir = unique_test_dir();
        let path = dir.join("lqos_overrides.lock");
        let _config_guard =
            TestLockConfigGuard::set(test_config_for(&dir, "save operator overrides"));

        {
            let _lock = FileLock::new_for_operation("save operator overrides")
                .expect("failed to create overrides temp lock");
            let contents = read_to_string(&path).expect("failed to read overrides temp lock");

            assert!(contents.contains(&format!("pid={}", std::process::id())));
            assert!(contents.contains("operation=save operator overrides"));
            assert!(contents.contains("created_unix="));
        }

        assert!(!path.exists());
        remove_dir_all(&dir).expect("failed to clean up temp test dir");
    }
}
