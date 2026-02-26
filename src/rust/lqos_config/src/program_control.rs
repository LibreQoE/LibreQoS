use std::{
    path::{Path, PathBuf},
    process::Command,
};
use thiserror::Error;
use tracing::error;

const PYTHON_PATH: &str = "/usr/bin/python3";

fn path_to_libreqos() -> Result<PathBuf, ProgramControlError> {
    let cfg = crate::load_config().map_err(|_| ProgramControlError::ConfigLoadError)?;
    let base_path = Path::new(&cfg.lqos_directory);
    Ok(base_path.join("LibreQoS.py"))
}

fn working_directory() -> Result<PathBuf, ProgramControlError> {
    let cfg = crate::load_config().map_err(|_| ProgramControlError::ConfigLoadError)?;
    let base_path = Path::new(&cfg.lqos_directory);
    Ok(base_path.to_path_buf())
}

/// Shells out and reloads the `LibreQos.py` program, storing all
/// emitted text and returning it.
pub fn load_libreqos() -> Result<String, ProgramControlError> {
    let path = path_to_libreqos()?;
    if !path.exists() {
        error!(
            "Unable to locate LibreQoS.py. ({}) Check your configuration directory.",
            path.display()
        );
        return Err(ProgramControlError::LibreQosPyNotFound);
    }
    if !Path::new(PYTHON_PATH).exists() {
        error!("Unable to find Python binary ({PYTHON_PATH})");
        return Err(ProgramControlError::PythonNotFound);
    }

    let reload_result = Command::new(PYTHON_PATH)
        .current_dir(working_directory()?)
        .arg("LibreQoS.py")
        .output()
        .map_err(|_| ProgramControlError::CommandFailed)?;
    let reload_stdout =
        String::from_utf8(reload_result.stdout).map_err(|_| ProgramControlError::StdInErrAccess)?;
    let reload_stderr =
        String::from_utf8(reload_result.stderr).map_err(|_| ProgramControlError::StdInErrAccess)?;

    let mut result_display = reload_stdout + &reload_stderr;
    if !reload_result.status.success() {
        let status = reload_result.status.to_string();
        error!("LibreQoS.py exited with status {status}");
        return Err(ProgramControlError::LibreQoSPyFailed {
            status,
            output: truncate_output(&result_display, 8192),
        });
    }

    // Also reload the scheduler service (best-effort: missing systemd/service should not block reload).
    result_display += "\n\nReloading Scheduler\n";
    match Command::new("/bin/systemctl")
        .arg("restart")
        .arg("lqos_scheduler")
        .output()
    {
        Ok(restart_result) => {
            let restart_stdout = String::from_utf8(restart_result.stdout)
                .map_err(|_| ProgramControlError::StdInErrAccess)?;
            let restart_stderr = String::from_utf8(restart_result.stderr)
                .map_err(|_| ProgramControlError::StdInErrAccess)?;
            if !restart_result.status.success() {
                let status = restart_result.status.to_string();
                error!("systemctl restart lqos_scheduler exited with status {status}");
                result_display += &format!("Scheduler restart failed: {status}\n");
            }
            result_display += &restart_stdout;
            result_display += &restart_stderr;
        }
        Err(e) => {
            error!("Failed to restart lqos_scheduler via systemctl: {e}");
            result_display += &format!("Scheduler restart failed to run: {e}\n");
        }
    }

    Ok(result_display)
}

/// Returns a best-effort truncated version of `s` limited to `max_len` bytes.
///
/// This function is pure: it has no side effects.
fn truncate_output(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }
    let start = s.len().saturating_sub(max_len);
    let mut out = String::from("…");
    out.push_str(&s[start..]);
    out
}

#[derive(Error, Debug)]
pub enum ProgramControlError {
    #[error("Unable to load lqos configuration from /etc")]
    ConfigLoadError,
    #[error("Unable to find LibreQoS.py")]
    LibreQosPyNotFound,
    #[error("Unable to find Python. Is it installed?")]
    PythonNotFound,
    #[error("Problem Invoking Command. This shouldn't happen.")]
    CommandFailed,
    #[error("Problem accessing stdin/stderr. This shouldn't happen")]
    StdInErrAccess,
    #[error("LibreQoS.py failed (status {status}): {output}")]
    LibreQoSPyFailed { status: String, output: String },
}
