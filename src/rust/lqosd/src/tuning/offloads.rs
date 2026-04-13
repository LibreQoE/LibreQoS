//! Host tuning commands for sysctls, NIC offloads, and CPU governor policy.

use lqos_config::Tunables;
use std::{io::ErrorKind, process::Command};
use tracing::warn;

pub fn bpf_sysctls() {
    let _ = Command::new("/sbin/sysctl")
        .arg("net.core.bpf_jit_enable=1")
        .output();
}

pub fn set_cpu_governor_performance() {
    for cpupower in ["/usr/bin/cpupower", "/usr/sbin/cpupower", "cpupower"] {
        match Command::new(cpupower)
            .args(["frequency-set", "--governor", "performance"])
            .output()
        {
            Ok(output) if output.status.success() => return,
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                if stderr.is_empty() {
                    warn!(
                        command = cpupower,
                        status = %output.status,
                        "Failed to set CPU governor to performance"
                    );
                } else {
                    warn!(
                        command = cpupower,
                        status = %output.status,
                        stderr = %stderr,
                        "Failed to set CPU governor to performance"
                    );
                }
                return;
            }
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => {
                warn!(
                    command = cpupower,
                    error = %error,
                    "Failed to run cpupower to set CPU governor to performance"
                );
                return;
            }
        }
    }

    warn!("Unable to set CPU governor to performance because cpupower was not found");
}

pub fn stop_irq_balance() {
    let _ = Command::new("/bin/systemctl")
        .args(["stop", "irqbalance"])
        .output();
}

pub fn netdev_budget(usecs: u32, packets: u32) {
    let _ = Command::new("/sbin/sysctl")
        .arg(format!("net.core.netdev_budget_usecs={usecs}"))
        .output();

    let _ = Command::new("/sbin/sysctl")
        .arg(format!("net.core.netdev_budget={packets}"))
        .output();
}

fn disable_individual_offload(interface: &str, feature: &str) {
    let _ = Command::new("/sbin/ethtool")
        .args(["--offload", interface, feature, "off"])
        .output();
}

pub fn ethtool_tweaks(interface: &str, tuning: &Tunables) {
    // Disabling individually to avoid complaints that a card doesn't support a feature anyway
    for feature in tuning.disable_offload.iter() {
        disable_individual_offload(interface, feature);
    }

    let _ = Command::new("/sbin/ethtool")
        .args([
            "-C",
            interface,
            "rx-usecs",
            &format!("\"{}\"", tuning.rx_usecs),
        ])
        .output();

    let _ = Command::new("/sbin/ethtool")
        .args([
            "-C",
            interface,
            "tx-usecs",
            &format!("\"{}\"", tuning.tx_usecs),
        ])
        .output();

    if tuning.disable_rxvlan {
        let _ = Command::new("/sbin/ethtool")
            .args(["-K", interface, "rxvlan", "off"])
            .output();
    }

    if tuning.disable_txvlan {
        let _ = Command::new("/sbin/ethtool")
            .args(["-K", interface, "txvlan", "off"])
            .output();
    }
}
