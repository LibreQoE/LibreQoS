use std::{path::{PathBuf, Path}, process::Command};
use anyhow::{Result, Error};
use crate::etc;

const PYTHON_PATH: &str = "/usr/bin/python3";

fn path_to_libreqos() -> Result<PathBuf> {
    let cfg = etc::EtcLqos::load()?;
    let base_path = Path::new(&cfg.lqos_directory);
    Ok(base_path.join("LibreQoS.py"))
}

fn working_directory() -> Result<PathBuf> {
    let cfg = etc::EtcLqos::load()?;
    let base_path = Path::new(&cfg.lqos_directory);
    Ok(base_path.to_path_buf())
}

pub fn load_libreqos() -> Result<String> {
    let path = path_to_libreqos()?;
    if !path.exists() {
        return Err(Error::msg("LibreQoS.py not found"));
    }
    if !Path::new(PYTHON_PATH).exists() {
        return Err(Error::msg("Python not found"));
    }

    let result = Command::new(PYTHON_PATH)
        .current_dir(working_directory()?)
        .arg("LibreQoS.py")
        .output()?;
    let stdout = String::from_utf8(result.stdout)?;
    let stderr = String::from_utf8(result.stderr)?;
    Ok(stdout + &stderr)
}