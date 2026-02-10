use anyhow::{Error, Result};
use nix::libc::{getpid, mode_t};
use std::{
    ffi::CString,
    fs::{File, remove_file},
    io::{Read, Write},
    path::Path,
};
use sysinfo::System;

const LOCK_PATH: &str = "/run/lqos/lqosd.lock";
const LOCK_DIR: &str = "/run/lqos";
const LOCK_DIR_PERMS: &str = "/run/lqos";

pub struct FileLock {}

impl FileLock {
    pub fn new() -> Result<Self> {
        Self::check_directory()?;
        let lock_path = Path::new(LOCK_PATH);
        if lock_path.exists() {
            if Self::is_lock_valid()? {
                return Err(Error::msg("lqosd is already running"));
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
        let pid: i32 = contents.parse()?;

        let sys = System::new_all();
        let pid = sysinfo::Pid::from(pid as usize);
        if let Some(process) = sys.processes().get(&pid) {
            if process
                .name()
                .to_str()
                .unwrap_or_default()
                .contains("lqosd")
            {
                return Ok(true);
            }
        }
        Ok(false)
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
