const TC_COMMAND: &str = "/sbin/tc";

/// Clear all traffic control queues for a given network interface.
/// 
/// # Arguments
/// * `interface` - The name of the network interface to clear queues for.
/// 
/// # Returns
/// * `Result<(), anyhow::Error>` - Returns Ok if successful, or an error if the command fails.
pub fn clear_all_queues(interface: &str) -> anyhow::Result<()> {
    let output = std::process::Command::new(TC_COMMAND)
        .arg("qdisc")
        .arg("delete")
        .arg("dev")
        .arg(interface)
        .arg("root")
        .output()?;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "Failed to clear queues: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(())
}

/// Check if the Multi-Queue (MQ) discipline is installed on a given network interface.
/// 
/// # Arguments
/// * `interface` - The name of the network interface to check for MQ installation.
/// 
/// # Returns
/// * `Result<bool, anyhow::Error>` - Returns Ok(true) if MQ is installed, Ok(false) if not, or an error if the command fails.
pub fn is_mq_installed(interface: &str) -> anyhow::Result<bool> {
    let output = std::process::Command::new(TC_COMMAND)
        .arg("qdisc")
        .arg("show")
        .arg("dev")
        .arg(interface)
        .arg("root")
        .output()?;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "Failed to check MQ installation: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let output_str = String::from_utf8_lossy(&output.stdout);
    Ok(output_str.contains("mq"))
}

/// Replace the Multi-Queue (MQ) discipline on a given network interface.
/// 
/// # Arguments
/// * `interface` - The name of the network interface on which to replace the MQ discipline.
/// 
/// # Returns
/// * `Result<(), anyhow::Error>` - Returns Ok if the replacement is successful, or an error if the command fails.
pub fn replace_mq(interface: &str) -> anyhow::Result<()> {
    // command = 'qdisc replace dev ' + thisInterface + ' root handle 7FFF: mq'
    let output = std::process::Command::new(TC_COMMAND)
        .arg("qdisc")
        .arg("replace")
        .arg("dev")
        .arg(interface)
        .arg("root")
        .arg("handle")
        .arg("7FFF:")
        .arg("mq")
        .output()?;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "Failed to replace MQ: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

pub fn make_top_htb(interface: &str, queue: u32) -> anyhow::Result<()> {
    // 'qdisc add dev ' + thisInterface + ' parent 7FFF:' + hex(queue+1) + ' handle ' + hex(queue+1) + ': htb default 2'
    let queue_hex = format!("0x{:x}", queue + 1);
    let queue_hex_colon = format!("{}:", queue_hex);
    let output = std::process::Command::new(TC_COMMAND)
        .args(&[
            "qdisc", "add", "dev", interface, "parent", "7FFF:", &queue_hex, "handle", &queue_hex_colon, "htb", "default", "2"
        ])
        .output()?;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "Failed to replace MQ: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

pub fn quantum(mbps: u64) -> u64 {
    const MIN_QUANTUM: u64 = 1522;
    let rate_in_bytes_per_second = mbps * 125_000;
    let quantum = u64::max(MIN_QUANTUM, rate_in_bytes_per_second / 8); // Assuming R2Q is 8
    quantum
}

pub fn make_parent_class(interface: &str, queue: u32, mbps: u64) -> anyhow::Result<()> {
    // 'class add dev ' + thisInterface + ' parent ' + hex(queue+1) + ': classid ' + hex(queue+1) + ':1 htb rate '+ str(upstream_bandwidth_capacity_download_mbps()) + 'mbit ceil ' + str(upstream_bandwidth_capacity_download_mbps()) + 'mbit' + quantum(upstream_bandwidth_capacity_download_mbps())
    let queue_hex_colon = format!("0x{:x}:", queue + 1);
    let queue_hex_colon1 = format!("0x{:x}:1", queue + 1);
    let mbps_string = format!("{}mbit", mbps);
    let quantum = quantum(mbps);
    let output = std::process::Command::new(TC_COMMAND)
        .args(&[
            "class", "add", "dev", interface, "parent", &queue_hex_colon, "classid", &queue_hex_colon1, 
            "htb", "rate", &mbps_string, "ceil", &mbps_string, "quantum", &quantum.to_string()
        ])
        .output()?;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "Failed to add class: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

pub fn make_default_sqm_bucket(interface: &str, queue: u32, sqm:&[&str]) -> anyhow::Result<()> {
    // command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':1 ' + sqm()
    let queue_hex_colon_one = format!("0x{:x}:1", queue + 1);
    let mut args = vec!["qdisc", "add", "dev", interface, "parent", &queue_hex_colon_one];
    args.extend_from_slice(sqm);
    let output = std::process::Command::new(TC_COMMAND)
        .args(&args)
        .output()?;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "Failed to add class: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

pub fn make_default_class(interface: &str, queue: u32, mbps: u64) -> anyhow::Result<()> {
    // 'class add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':1 classid ' + hex(queue+1) + ':2 htb rate ' + str(round((upstream_bandwidth_capacity_download_mbps()-1)/4)) + 'mbit ceil ' + str(upstream_bandwidth_capacity_download_mbps()-1) + 'mbit prio 5' + quantum(upstream_bandwidth_capacity_download_mbps())
    let queue_hex_colon = format!("0x{:x}:1", queue + 1);
    let queue_hex_colon2 = format!("0x{:x}:2", queue + 1);
    let mbps_quarter: f32 = (mbps as f32 - 1.0) / 4.0;
    let quantum = quantum(mbps);
    let output = std::process::Command::new(TC_COMMAND)
        .args(&[
            "class", "add", "dev", interface, "parent", &queue_hex_colon,
            "classid", &queue_hex_colon2, "htb", "rate", &format!("{mbps_quarter:.2}mbit"),
            "ceil", &format!("{mbps_quarter:.2}mbit"), "prio", "5",
            "quantum", &quantum.to_string()
        ])
        .output()?;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "Failed to add class: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

pub fn make_default_class_sqm(interface: &str, queue: u32, sqm:&[&str]) -> anyhow::Result<()> {
    // 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':2 ' + sqm()
    let queue_hex_colon_two = format!("0x{:x}:2", queue + 1);
    let mut args = vec!["qdisc", "add", "dev", interface, "parent", &queue_hex_colon_two];
    args.extend_from_slice(sqm);
    let output = std::process::Command::new(TC_COMMAND)
        .args(&args)
        .output()?;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "Failed to add class: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}