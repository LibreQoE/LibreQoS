//! Optional `ShapedDevices.csv` access helpers.

use std::path::PathBuf;

use anyhow::Result;
use lqos_config::ConfigShapedDevices;

/// Return the configured `ShapedDevices.csv` path.
///
/// This reads `/etc/lqos.conf` through `lqos_config`.
pub fn path() -> Result<PathBuf> {
    Ok(ConfigShapedDevices::path()?)
}

/// Load `ShapedDevices.csv` if it exists.
///
/// Missing CSV input is valid for integration-backed installs, so callers get
/// `Ok(None)` instead of a startup or request error.
pub fn load_optional() -> Result<Option<ConfigShapedDevices>> {
    let path = ConfigShapedDevices::path()?;
    if !path.exists() {
        return Ok(None);
    }

    Ok(ConfigShapedDevices::load().map(Some)?)
}

/// Load `ShapedDevices.csv`, returning an empty device list when it is absent.
pub fn load_or_empty() -> Result<ConfigShapedDevices> {
    Ok(load_optional()?.unwrap_or_default())
}
