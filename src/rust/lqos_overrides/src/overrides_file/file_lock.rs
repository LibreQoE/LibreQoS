use std::{ffi::CString, fs::{remove_file, File}, io::{Read, Write}, path::Path};
use anyhow::{Error, Result};
use nix::{errno::Errno, libc::{getpid, mode_t}};

const LOCK_PATH: &str = "/run/lqos/lqos_overrides.lock";
const LOCK_DIR: &str = "/run/lqos";
const LOCK_DIR_PERMS: &str = "/run/lqos";

pub struct FileLock {

}

impl FileLock {
    pub fn new() -> Result<Self> {
        Self::check_directory()?;
        let lock_path = Path::new(LOCK_PATH);
        if lock_path.exists() {
            if Self::is_lock_valid()? {
                return Err(Error::msg("The lqos_overrides file is locked by another process."));
            }

            // It's a stale pid, so we need to replace it
            Self::create_lock()?;

            Ok(Self {})
        } else {
            Self::create_lock()?;
            Ok(Self {})
        }
    }

    fn is_lock_valid() -> Result<bool> {
        let mut f = File::open(LOCK_PATH)?;
        let mut contents = String::new();
        f.read_to_string(&mut contents)?;
        let pid: i32 = contents.trim().parse()?;

        // Check if the process exists by sending signal 0.
        // Returns 0 if the process exists; -1 with ESRCH if it doesn't;
        // other errors (e.g., EPERM) mean the process exists but we lack permission.
        let ret = unsafe { nix::libc::kill(pid, 0) };
        if ret == 0 {
            return Ok(true);
        }
        let err = Errno::last();
        if err == Errno::ESRCH {
            Ok(false)
        } else {
            Ok(true)
        }
    }

    fn create_lock() -> Result<()> {
        let pid = unsafe { getpid() };
        let pid_format = format!("{pid}");
        {
            let mut f = File::create(LOCK_PATH)?;
            f.write_all(pid_format.as_bytes())?;
        }
        let unix_path = CString::new(LOCK_PATH)?;
        unsafe {
            nix::libc::chmod(unix_path.as_ptr(), mode_t::from_le(666));
        }
        Ok(())
    }

    fn check_directory() -> Result<()> {
        let dir_path = std::path::Path::new(LOCK_DIR);
        if dir_path.exists() && dir_path.is_dir() {
            Ok(())
        } else {
            std::fs::create_dir(dir_path)?;
            let unix_path = CString::new(LOCK_DIR_PERMS)?;
            unsafe {
                nix::libc::chmod(unix_path.as_ptr(), 777);
            }
            Ok(())
        }
    }

    pub fn remove_lock() {
        let _ = remove_file(LOCK_PATH); // Ignore result
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        Self::remove_lock();
    }
}
