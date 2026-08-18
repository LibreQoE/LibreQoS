//! Ubuntu 24.04 systemd hotfix helpers for first-run setup.

use anyhow::{Context, Result, bail};
use std::process::Command;

const HOTFIX_SCRIPT: &str = "/opt/libreqos/src/systemd_hotfix.sh";

/// Current systemd hotfix state for operator-facing setup surfaces.
pub struct HotfixStatus {
    pub required: bool,
    pub detail: String,
}

/// Result of running the hotfix installer.
pub struct HotfixInstallResult {
    pub summary: String,
    pub detail: String,
}

fn script_exists() -> bool {
    std::path::Path::new(HOTFIX_SCRIPT).exists()
}

/// Returns the current hotfix status by invoking the shipped helper script.
///
/// Side effects: executes `systemd_hotfix.sh status`.
pub fn status() -> Result<HotfixStatus> {
    if !script_exists() {
        bail!("Hotfix helper script not found at {HOTFIX_SCRIPT}.");
    }

    let output = Command::new(HOTFIX_SCRIPT)
        .arg("status")
        .output()
        .with_context(|| format!("Unable to run {HOTFIX_SCRIPT} status"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let detail = if !stdout.is_empty() {
        stdout
    } else if !stderr.is_empty() {
        stderr
    } else if output.status.success() {
        "Hotfix is not required.".to_string()
    } else {
        "Hotfix helper did not return any status text.".to_string()
    };

    Ok(HotfixStatus {
        required: output.status.success(),
        detail,
    })
}

/// Installs the Ubuntu 24.04 systemd hotfix without prompting for reboot.
///
/// Side effects: executes `systemd_hotfix.sh install`, which can modify APT
/// sources, install packages, and write the marker file.
pub fn install() -> Result<HotfixInstallResult> {
    if !script_exists() {
        bail!("Hotfix helper script not found at {HOTFIX_SCRIPT}.");
    }

    let output = Command::new(HOTFIX_SCRIPT)
        .arg("install")
        .env("HOTFIX_SKIP_REBOOT_PROMPT", "1")
        .output()
        .with_context(|| format!("Unable to run {HOTFIX_SCRIPT} install"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() {
        let detail = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("Hotfix installer exited with status {}", output.status)
        };
        bail!("{detail}");
    }

    let detail = if !stdout.is_empty() {
        stdout
    } else {
        "Hotfix installed.".to_string()
    };
    let summary = match status() {
        Ok(current) if !current.required => "Hotfix installed successfully.".to_string(),
        Ok(_) => "Hotfix installer completed, but the host still reports the hotfix as required."
            .to_string(),
        Err(_) => "Hotfix installer completed. Setup checks were refreshed.".to_string(),
    };

    Ok(HotfixInstallResult { summary, detail })
}
