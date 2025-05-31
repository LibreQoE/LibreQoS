//! Traffic Control (TC) command execution for LibreQoS
//! 
//! This module mirrors LibreQoS.py's TC functionality with two distinct queue types:
//! 
//! 1. **Structural Queues** (Sites/APs from network.json)
//!    - Intermediate nodes in the HTB hierarchy
//!    - Only have HTB classes, NO qdiscs
//!    - Tracked by site_hash (i64 hash of site name)
//!    - Created with `add_structural_htb_class()`
//! 
//! 2. **Circuit Queues** (Customer circuits)
//!    - Leaf nodes that shape actual traffic
//!    - Have both HTB class AND CAKE/fq_codel qdisc
//!    - Tracked by circuit_hash (i64 hash of circuit ID)
//!    - Created with `add_circuit_htb_class()` + `add_circuit_qdisc()`
//! 
//! The hashes are used for tracking in Phase 2+ but accepted/ignored in Phase 1.

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

// === Circuit Management Functions ===

/// Generic HTB class creation (used by specific functions below)
/// 
/// Note: You should typically use one of:
/// - `add_circuit_htb_class()` for circuits (leaf nodes with CAKE)
/// - `add_structural_htb_class()` for sites/APs (intermediate nodes)
/// - Direct calls only for special cases
pub fn add_htb_class(
    interface: &str, 
    parent: &str, 
    classid: &str, 
    rate_mbps: f64, 
    ceil_mbps: f64, 
    prio: u32,
    r2q: u64,
    circuit_hash: Option<i64>, // Some(hash) for circuits, None for structural nodes
    comment: Option<&str>
) -> anyhow::Result<()> {
    // 'class add dev ' + interface + ' parent ' + parent + ' classid ' + classid + 
    // ' htb rate '+ rate + ' ceil '+ ceil + ' prio ' + prio + quantum + comment
    let rate_str = format_rate_for_tc(rate_mbps);
    let ceil_str = format_rate_for_tc(ceil_mbps);
    let quantum_val = quantum(ceil_mbps, r2q); // Use ceil for quantum like Python
    let prio_str = prio.to_string();
    let quantum_str = quantum_val.to_string();
    
    let args = vec![
        "class", "add", "dev", interface, "parent", parent, "classid", classid,
        "htb", "rate", &rate_str, "ceil", &ceil_str, "prio", &prio_str,
        "quantum", &quantum_str
    ];
    
    // Store circuit_hash for future use (Phase 2+)
    if let Some(_hash) = circuit_hash {
        // Will be used for tracking circuits in later phases
    }
    
    // Add comment if provided (though TC doesn't actually support comments)
    if let Some(_comment) = comment {
        // TC doesn't support comments in the command, but we include this for compatibility
    }
    
    execute_tc_command(&args)
}

/// Add a qdisc (CAKE/fq_codel) to a circuit class
/// 
/// This is ONLY for circuits (leaf nodes). Structural nodes (sites/APs) 
/// do NOT get qdiscs - they only have HTB classes for hierarchy.
pub fn add_circuit_qdisc(
    interface: &str,
    parent_major: u32,
    parent_minor: u32,
    circuit_hash: i64, // Required for circuits
    sqm_params: &[&str]
) -> anyhow::Result<()> {
    // 'qdisc add dev ' + interface + ' parent ' + major + ':' + minor + ' ' + sqm
    let parent = format!("{:x}:{:x}", parent_major, parent_minor);
    let mut args = vec!["qdisc", "add", "dev", interface, "parent", &parent];
    args.extend_from_slice(sqm_params);
    
    // Store circuit_hash for future use (Phase 2+)
    let _ = circuit_hash; // Will be used for tracking circuits in later phases
    
    execute_tc_command(&args)
}

/// Delete all queues on an interface (used in clearPriorSettings)
pub fn delete_root_qdisc(interface: &str) -> anyhow::Result<()> {
    // 'tc qdisc delete dev ' + interface + ' root'
    execute_tc_command(&["qdisc", "delete", "dev", interface, "root"])
}

/// Check if MQ is installed on an interface (for clearPriorSettings logic)
pub fn has_mq_qdisc(interface: &str) -> anyhow::Result<bool> {
    // This is already implemented as is_mq_installed
    is_mq_installed(interface)
}

/// Create an HTB class specifically for a circuit (with CAKE shaper)
/// Circuits are leaf nodes that shape actual customer traffic
pub fn add_circuit_htb_class(
    interface: &str,
    parent: &str,
    classid: &str,
    rate_mbps: f64,
    ceil_mbps: f64,
    circuit_hash: i64,
    comment: Option<&str>,
    r2q: u64,
) -> anyhow::Result<()> {
    add_htb_class(
        interface,
        parent,
        classid,
        rate_mbps,
        ceil_mbps,
        3, // Priority 3 for circuits (from Python)
        r2q,
        Some(circuit_hash),
        comment,
    )
}

/// Create an HTB class for a structural node (site/AP from network.json)
/// These are intermediate nodes in the hierarchy, NOT leaf nodes
pub fn add_structural_htb_class(
    interface: &str,
    parent: &str,
    classid: &str,
    rate_mbps: f64,
    ceil_mbps: f64,
    site_hash: i64,  // Hash of site name from network.json
    r2q: u64,
) -> anyhow::Result<()> {
    // Store site_hash for future tracking (Phase 2+)
    let _ = site_hash;
    
    add_htb_class(
        interface,
        parent,
        classid,
        rate_mbps,
        ceil_mbps,
        3, // Priority 3 for structural nodes (from Python)
        r2q,
        None, // Not a circuit
        None, // No comment
    )
}

/// Apply CAKE RTT adjustments for low bandwidth circuits
/// This matches Python's sqmFixupRate function but handles fractional speeds
pub fn sqm_fixup_rate(rate_mbps: f64, sqm: &str) -> String {
    // If we aren't using cake, just return the sqm string
    if !sqm.starts_with("cake") || sqm.contains("rtt") {
        return sqm.to_string();
    }
    
    // Based on: 1 MTU is 1500 bytes, or 12,000 bits.
    // At 1 Mbps, (1,000 bits per ms) transmitting an MTU takes 12ms. Add 3ms for overhead, and we get 15ms.
    // So 15ms divided by 5 (for 1%) multiplied by 100 yields 300ms.
    
    // Use ranges for fractional speed support
    let rtt_suffix = if rate_mbps <= 1.5 {
        " rtt 300ms"
    } else if rate_mbps <= 2.5 {
        " rtt 180ms" 
    } else if rate_mbps <= 3.5 {
        " rtt 140ms"
    } else if rate_mbps <= 4.5 {
        " rtt 120ms"
    } else {
        ""
    };
    
    format!("{}{}", sqm, rtt_suffix)
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
    
    #[test]
    fn test_sqm_fixup_rate() {
        // Test SQM fixup for CAKE at low rates
        let cake_base = "cake bandwidth 5mbit";
        
        // Test exact and fractional rates
        assert_eq!(sqm_fixup_rate(1.0, cake_base), "cake bandwidth 5mbit rtt 300ms");
        assert_eq!(sqm_fixup_rate(1.5, cake_base), "cake bandwidth 5mbit rtt 300ms");
        assert_eq!(sqm_fixup_rate(2.0, cake_base), "cake bandwidth 5mbit rtt 180ms");
        assert_eq!(sqm_fixup_rate(2.5, cake_base), "cake bandwidth 5mbit rtt 180ms");
        assert_eq!(sqm_fixup_rate(3.0, cake_base), "cake bandwidth 5mbit rtt 140ms");
        assert_eq!(sqm_fixup_rate(3.5, cake_base), "cake bandwidth 5mbit rtt 140ms");
        assert_eq!(sqm_fixup_rate(4.0, cake_base), "cake bandwidth 5mbit rtt 120ms");
        assert_eq!(sqm_fixup_rate(4.5, cake_base), "cake bandwidth 5mbit rtt 120ms");
        assert_eq!(sqm_fixup_rate(5.0, cake_base), "cake bandwidth 5mbit");
        
        // Test non-CAKE qdiscs (should return unchanged)
        let fq_codel = "fq_codel";
        assert_eq!(sqm_fixup_rate(1.0, fq_codel), "fq_codel");
        
        // Test CAKE with RTT already present (should return unchanged)
        let cake_with_rtt = "cake bandwidth 5mbit rtt 100ms";
        assert_eq!(sqm_fixup_rate(1.0, cake_with_rtt), "cake bandwidth 5mbit rtt 100ms");
    }
    
    #[test]
    fn test_structural_vs_circuit_creation() {
        // This test demonstrates the distinction between structural and circuit queues
        
        let site_hash: i64 = 987654321;  // Hash of "Site-A" or similar
        let circuit_hash: i64 = 1234567890; // Hash of circuit ID
        
        // 1. Structural node (site/AP) - only HTB class, no qdisc
        let _ = add_structural_htb_class(
            "eth0",
            "1:1",      // Parent is root or another structural node
            "1:10",     // This site's class
            100.0,      // Site's total bandwidth
            100.0,
            site_hash,
            21,
        );
        
        // 2. Circuit (leaf) - HTB class + CAKE qdisc
        let _ = add_circuit_htb_class(
            "eth0",
            "1:10",     // Parent is the site class above
            "1:100",    // Circuit's class
            10.5,       // Circuit rate (fractional)
            15.0,       // Circuit ceil
            circuit_hash,
            Some("Customer ABC"),
            21,
        );
        
        // Only circuits get qdiscs (CAKE shaper)
        let _ = add_circuit_qdisc(
            "eth0",
            1,
            100,
            circuit_hash,
            &["cake", "bandwidth", "15mbit"],
        );
        
        // Structural nodes do NOT get qdiscs - they're just HTB hierarchy
    }
}