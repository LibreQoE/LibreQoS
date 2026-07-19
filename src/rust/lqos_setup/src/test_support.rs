use once_cell::sync::Lazy;
use parking_lot::{Mutex, MutexGuard};

static CONFIG_ENV_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

pub(crate) struct ConfigEnvGuard {
    _lock: MutexGuard<'static, ()>,
    previous_lqos_config: Option<std::ffi::OsString>,
}

impl ConfigEnvGuard {
    pub(crate) fn set_lqos_config(path: &std::path::Path) -> Self {
        let lock = CONFIG_ENV_LOCK.lock();
        let previous_lqos_config = std::env::var_os("LQOS_CONFIG");
        unsafe {
            std::env::set_var("LQOS_CONFIG", path);
        }
        lqos_config::clear_cached_config();
        Self {
            _lock: lock,
            previous_lqos_config,
        }
    }
}

impl Drop for ConfigEnvGuard {
    fn drop(&mut self) {
        match self.previous_lqos_config.take() {
            Some(value) => unsafe { std::env::set_var("LQOS_CONFIG", value) },
            None => unsafe { std::env::remove_var("LQOS_CONFIG") },
        }
        lqos_config::clear_cached_config();
    }
}
