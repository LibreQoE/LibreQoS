use anyhow::{Error, Result};
use nix::{
    errno::Errno,
    libc::{getpid, mode_t},
};
use std::{
    ffi::CString,
    fs::{File, remove_file},
    io::{Read, Write},
    path::Path,
};

const LOCK_PATH: &str = "/run/lqos/lqos_overrides.lock";
const LOCK_DIR: &str = "/run/lqos";
const LOCK_DIR_PERMS: &str = "/run/lqos";

/// Cross-process lock used while mutating operator-owned override files.
pub struct FileLock {}

impl FileLock {
    /// Acquires the lock or returns an error if another live process holds it.
    pub fn new() -> Result<Self> {
        Self::check_directory()?;
        let lock_path = Path::new(LOCK_PATH);
        if lock_path.exists() {
            if Self::is_lock_valid()? {
                return Err(Error::msg(
                    "The LibreQoS overrides files are locked by another process.",
                ));
            }

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

    /// Removes the current lock file, if present.
    pub fn remove_lock() {
        let _ = remove_file(LOCK_PATH);
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        Self::remove_lock();
    }
}
