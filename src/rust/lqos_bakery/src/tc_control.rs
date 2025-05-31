use std::fs::OpenOptions;
use std::io::Write;

const TC_COMMAND: &str = "/sbin/tc";
const WRITE_TC_TO_FILE: bool = false; // Set to true for testing/comparison
const TC_OUTPUT_FILE: &str = "tc-rust.txt";

/// Execute a TC command or write it to a file for testing
/// 
/// # Arguments
/// * `args` - The TC command arguments (everything after /sbin/tc)
/// 
/// # Returns
/// * `Result<(), anyhow::Error>` - Returns Ok if successful, or an error if the command fails.
fn execute_tc_command(args: &[&str]) -> anyhow::Result<()> {
    if WRITE_TC_TO_FILE {
        // Write to file for comparison testing
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(TC_OUTPUT_FILE)?;
        
        // Write just the arguments (no /sbin/tc prefix) to match Python format
        writeln!(file, "{}", args.join(" "))?;
        Ok(())
    } else {
        // Execute the actual command
        let output = std::process::Command::new(TC_COMMAND)
            .args(args)
            .output()?;

        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "TC command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok(())
    }
}

/// Clear all traffic control queues for a given network interface.
/// 
/// # Arguments
/// * `interface` - The name of the network interface to clear queues for.
/// 
/// # Returns
/// * `Result<(), anyhow::Error>` - Returns Ok if successful, or an error if the command fails.
pub fn clear_all_queues(interface: &str) -> anyhow::Result<()> {
    execute_tc_command(&["qdisc", "delete", "dev", interface, "root"])
}

/// Check if the Multi-Queue (MQ) discipline is installed on a given network interface.
/// 
/// # Arguments
/// * `interface` - The name of the network interface to check for MQ installation.
/// 
/// # Returns
/// * `Result<bool, anyhow::Error>` - Returns Ok(true) if MQ is installed, Ok(false) if not, or an error if the command fails.
pub fn is_mq_installed(interface: &str) -> anyhow::Result<bool> {
    // This function needs to read output, so it can't use execute_tc_command
    if WRITE_TC_TO_FILE {
        // In test mode, we can't actually check, so return a dummy value
        return Ok(false);
    }
    
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
    execute_tc_command(&["qdisc", "replace", "dev", interface, "root", "handle", "7FFF:", "mq"])
}

pub fn make_top_htb(interface: &str, queue: u32) -> anyhow::Result<()> {
    // 'qdisc add dev ' + thisInterface + ' parent 7FFF:' + hex(queue+1) + ' handle ' + hex(queue+1) + ': htb default 2'
    let queue_hex = format!("{:x}", queue + 1);
    let parent = format!("7FFF:{}", queue_hex);
    let handle = format!("{}:", queue_hex);
    
    execute_tc_command(&[
        "qdisc", "add", "dev", interface, "parent", &parent, "handle", &handle, "htb", "default", "2"
    ])
}

/// Format a rate in Mbps for TC commands with smart unit selection.
/// This matches Python's format_rate_for_tc function.
/// - Rates >= 1000 Mbps use 'gbit' 
/// - Rates >= 1 Mbps use 'mbit'
/// - Rates < 1 Mbps use 'kbit'
pub fn format_rate_for_tc(rate_mbps: f64) -> String {
    if rate_mbps >= 1000.0 {
        format!("{:.1}gbit", rate_mbps / 1000.0)
    } else if rate_mbps >= 1.0 {
        format!("{:.1}mbit", rate_mbps)
    } else {
        format!("{:.0}kbit", rate_mbps * 1000.0)
    }
}

/// Calculate quantum value based on bandwidth and r2q.
/// This matches Python's quantum function.
pub fn quantum(rate_mbps: f64, r2q: u64) -> u64 {
    const MIN_QUANTUM: u64 = 1522;
    let rate_in_bytes_per_second = (rate_mbps * 125_000.0) as u64;
    let quantum = u64::max(MIN_QUANTUM, rate_in_bytes_per_second / r2q);
    quantum
}

pub fn make_parent_class(interface: &str, queue: u32, mbps: f64, r2q: u64) -> anyhow::Result<()> {
    // 'class add dev ' + thisInterface + ' parent ' + hex(queue+1) + ': classid ' + hex(queue+1) + ':1 htb rate '+ str(upstream_bandwidth_capacity_download_mbps()) + 'mbit ceil ' + str(upstream_bandwidth_capacity_download_mbps()) + 'mbit' + quantum(upstream_bandwidth_capacity_download_mbps())
    let parent = format!("{:x}:", queue + 1);
    let classid = format!("{:x}:1", queue + 1);
    let rate_string = format_rate_for_tc(mbps);
    let quantum_val = quantum(mbps, r2q);
    
    execute_tc_command(&[
        "class", "add", "dev", interface, "parent", &parent, "classid", &classid, 
        "htb", "rate", &rate_string, "ceil", &rate_string, "quantum", &quantum_val.to_string()
    ])
}

pub fn make_default_sqm_bucket(interface: &str, queue: u32, sqm:&[&str]) -> anyhow::Result<()> {
    // command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':1 ' + sqm()
    let parent = format!("{:x}:1", queue + 1);
    let mut args = vec!["qdisc", "add", "dev", interface, "parent", &parent];
    args.extend_from_slice(sqm);
    execute_tc_command(&args)
}

pub fn make_default_class(interface: &str, queue: u32, mbps: f64, r2q: u64) -> anyhow::Result<()> {
    // 'class add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':1 classid ' + hex(queue+1) + ':2 htb rate ' + str(round((upstream_bandwidth_capacity_download_mbps()-1)/4)) + 'mbit ceil ' + str(upstream_bandwidth_capacity_download_mbps()-1) + 'mbit prio 5' + quantum(upstream_bandwidth_capacity_download_mbps())
    let parent = format!("{:x}:1", queue + 1);
    let classid = format!("{:x}:2", queue + 1);
    let mbps_quarter = (mbps - 1.0) / 4.0;
    let mbps_minus_one = mbps - 1.0;
    let quantum_val = quantum(mbps, r2q);
    
    execute_tc_command(&[
        "class", "add", "dev", interface, "parent", &parent,
        "classid", &classid, "htb", "rate", &format_rate_for_tc(mbps_quarter),
        "ceil", &format_rate_for_tc(mbps_minus_one), "prio", "5",
        "quantum", &quantum_val.to_string()
    ])
}

pub fn make_default_class_sqm(interface: &str, queue: u32, sqm:&[&str]) -> anyhow::Result<()> {
    // 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':2 ' + sqm()
    let parent = format!("{:x}:2", queue + 1);
    let mut args = vec!["qdisc", "add", "dev", interface, "parent", &parent];
    args.extend_from_slice(sqm);
    execute_tc_command(&args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_rate_for_tc() {
        // Test exact matches with Python's format_rate_for_tc
        
        // Rates >= 1000 Mbps use gbit
        assert_eq!(format_rate_for_tc(1000.0), "1.0gbit");
        assert_eq!(format_rate_for_tc(1500.0), "1.5gbit");
        assert_eq!(format_rate_for_tc(10000.0), "10.0gbit");
        
        // Rates >= 1 Mbps use mbit
        assert_eq!(format_rate_for_tc(1.0), "1.0mbit");
        assert_eq!(format_rate_for_tc(1.5), "1.5mbit");
        assert_eq!(format_rate_for_tc(10.0), "10.0mbit");
        assert_eq!(format_rate_for_tc(999.9), "999.9mbit");
        
        // Rates < 1 Mbps use kbit
        assert_eq!(format_rate_for_tc(0.5), "500kbit");
        assert_eq!(format_rate_for_tc(0.1), "100kbit");
        assert_eq!(format_rate_for_tc(0.768), "768kbit");
    }

    #[test]
    fn test_quantum() {
        // Test quantum calculation with various r2q values
        // MIN_QUANTUM is 1522
        
        // Python: quantum(1.0, R2Q=10) -> 12500
        assert_eq!(quantum(1.0, 10), 12500);
        
        // Python: quantum(10.0, R2Q=10) -> 125000
        assert_eq!(quantum(10.0, 10), 125000);
        
        // Python: quantum(100.0, R2Q=10) -> 1250000
        assert_eq!(quantum(100.0, 10), 1250000);
        
        // Python: quantum(1000.0, R2Q=21) -> 5952380
        assert_eq!(quantum(1000.0, 21), 5952380);
        
        // Test fractional rates
        assert_eq!(quantum(1.5, 10), 18750);
        assert_eq!(quantum(0.5, 10), 6250); // Python returns 6250, not MIN_QUANTUM
    }
}