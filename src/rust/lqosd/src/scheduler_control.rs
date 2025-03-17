//! Controls the Python scheduler

// TODO: Add a Docker "start scheduler" function (see the `dockerization` branch).

use std::process::Command;
use anyhow::Result;

pub fn enable_scheduler() -> Result<()> {
    // TODO: we also need a Docker version
    Command::new("/bin/systemctl")
        .arg("enable")
        .arg("lqos_scheduler")
        .output()?;
    Ok(())
}

pub fn restart_scheduler() -> Result<()> {
    // TODO: we also need a Docker version
    Command::new("/bin/systemctl")
        .arg("restart")
        .arg("lqos_scheduler")
        .output()?;

    Ok(())
}