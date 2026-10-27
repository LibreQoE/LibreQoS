//! The I/O seams: the power core is PURE over [`Sysfs`] + [`Commands`] so the
//! platform mapping, the plan, and the reversibility logic are unit-testable
//! without touching a real host (`docs/power_profile_work_order.md` §6).

use std::collections::BTreeMap;
use std::io;

/// The sysfs (and small text-file) seam.
pub trait Sysfs {
    /// Read a text file, trimmed. `None` when absent/unreadable.
    fn read(&self, path: &str) -> Option<String>;
    /// Write `value` (a newline is appended) to `path`.
    fn write(&self, path: &str, value: &str) -> io::Result<()>;
    /// Directory entry NAMES (not full paths), unsorted.
    fn read_dir(&self, dir: &str) -> Vec<String>;

    /// `true` when the path is readable.
    fn exists(&self, path: &str) -> bool {
        self.read(path).is_some()
    }
}

/// The process seam (systemd / virtualization probes).
pub trait Commands {
    /// Run `prog` with `args`; `None` when the program is missing/not runnable.
    fn run(&self, prog: &str, args: &[&str]) -> Option<(i32, String)>;
}

/// The real sysfs. Writes go through a temp file + rename? No — sysfs is not a
/// regular filesystem and cannot be rename-targeted; a direct write is the only
/// correct call.
pub struct RealSysfs;

impl Sysfs for RealSysfs {
    fn read(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
    }

    fn write(&self, path: &str, value: &str) -> io::Result<()> {
        std::fs::write(path, format!("{value}\n"))
    }

    fn read_dir(&self, dir: &str) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect()
    }
}

/// A command runner that always reports "not present" (tests / dry-runs).
pub struct NoCommands;

impl Commands for NoCommands {
    fn run(&self, _prog: &str, _args: &[&str]) -> Option<(i32, String)> {
        None
    }
}

/// The real command runner.
pub struct RealCommands;

impl Commands for RealCommands {
    fn run(&self, prog: &str, args: &[&str]) -> Option<(i32, String)> {
        let out = std::process::Command::new(prog).args(args).output().ok()?;
        Some((
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
        ))
    }
}

/// An in-memory [`Sysfs`] for tests and dry-runs.
#[derive(Default, Clone)]
pub struct MemorySysfs {
    pub files: BTreeMap<String, String>,
    pub dirs: BTreeMap<String, Vec<String>>,
}

impl MemorySysfs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, path: &str, value: &str) -> &mut Self {
        self.files.insert(path.to_string(), value.to_string());
        self
    }

    pub fn dir(&mut self, path: &str, entries: &[&str]) -> &mut Self {
        self.dirs.insert(
            path.to_string(),
            entries.iter().map(|s| s.to_string()).collect(),
        );
        self
    }
}

impl Sysfs for MemorySysfs {
    fn read(&self, path: &str) -> Option<String> {
        self.files.get(path).cloned()
    }

    fn write(&self, _path: &str, _value: &str) -> io::Result<()> {
        // MemorySysfs is read-only: apply records the plan but never mutates it.
        Ok(())
    }

    fn read_dir(&self, dir: &str) -> Vec<String> {
        self.dirs.get(dir).cloned().unwrap_or_default()
    }
}
