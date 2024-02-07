use log::error;
use thiserror::Error;
use std::{
  path::{Path, PathBuf},
  process::Command,
};

const PYTHON_PATH: &str = "/usr/bin/python3";

fn path_to_libreqos() -> Result<PathBuf, ProgramControlError> {
  let cfg =
    crate::load_config().map_err(|_| ProgramControlError::ConfigLoadError)?;
  let base_path = Path::new(&cfg.lqos_directory);
  Ok(base_path.join("LibreQoS.py"))
}

fn working_directory() -> Result<PathBuf, ProgramControlError> {
  let cfg =
  crate::load_config().map_err(|_| ProgramControlError::ConfigLoadError)?;
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

  let result = Command::new(PYTHON_PATH)
    .current_dir(working_directory()?)
    .arg("LibreQoS.py")
    .output()
    .map_err(|_| ProgramControlError::CommandFailed)?;
  let stdout = String::from_utf8(result.stdout)
    .map_err(|_| ProgramControlError::StdInErrAccess)?;
  let stderr = String::from_utf8(result.stderr)
    .map_err(|_| ProgramControlError::StdInErrAccess)?;
  Ok(stdout + &stderr)
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
}
