use lqos_config::Tunables;
use tokio::process::Command;

pub async fn bpf_sysctls() {
    let _ = Command::new("/sbin/sysctl")
        .arg(format!("net.core.bpf_jit_enable=1"))
        .output().await;
}

pub async fn stop_irq_balance() {
    let _ = Command::new("/bin/systemctl")
        .args(["stop", "irqbalance"])
        .output().await;
}

pub async fn netdev_budget(usecs: u32, packets: u32) {
    let _ = Command::new("/sbin/sysctl")
        .arg(format!("net.core.netdev_budget_usecs={usecs}"))
        .output().await;

    let _ = Command::new("/sbin/sysctl")
        .arg(format!("net.core.netdev_budget={packets}"))
        .output().await;
}

async fn disable_individual_offload(interface: &str, feature: &str) {
    let _ = Command::new("/sbin/ethtool")
        .args(["--offload", interface, feature, "off"])
        .output().await;
}

pub async fn ethtool_tweaks(interface: &str, tuning: &Tunables) {
    // Disabling individually to avoid complaints that a card doesn't support a feature anyway
    for feature in tuning.disable_offload.iter() {
        disable_individual_offload(interface, feature).await;
    }
    
    let _ = Command::new("/sbin/ethtool")
        .args(["-C", interface, "rx-usecs", &format!("\"{}\"", tuning.rx_usecs)])
        .output().await;

    let _ = Command::new("/sbin/ethtool")
        .args(["-C", interface, "tx-usecs", &format!("\"{}\"", tuning.tx_usecs)])
        .output().await;

    if tuning.disable_rxvlan {
        let _ = Command::new("/sbin/ethtool")
            .args(["-K", interface, "rxvlan", "off"])
            .output().await;
    }

    if tuning.disable_txvlan {
        let _ = Command::new("/sbin/ethtool")
            .args(["-K", interface, "txvlan", "off"])
            .output().await;
    }
}