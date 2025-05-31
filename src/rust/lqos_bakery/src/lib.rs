//! The Bakery is where CAKE is made!
//! 
//! More specifically, this crate provides a tracker of TC queues - described by the LibreQoS.py process,
//! but tracked for changes.
//! 
//! In phase 1, the Bakery will build queues and a matching structure to track them. It will act exactly
//! like the LibreQoS.py process.
//! 
//! In phase 2, the Bakery will *not* create CAKE queues - just the HTB hierarchy. When circuits are
//! detected as having traffic, the associated queue will be created. Ideally, some form of timeout
//! will be used to remove queues that are no longer in use. (Saving resources)
//! 
//! In phase 3, the Bakery will - after initial creation - track the queues and update them as needed.
//! This will take a "diff" approach, finding differences and only applying those changes.
//! 
//! In phase 4, the Bakery will implement "live move" --- allowing queues to be moved losslessly. This will
//! complete the NLNet project goals.

#![deny(missing_docs)]

mod tc_control;

use std::path::Path;

use crossbeam_channel::Receiver;
use tracing::error;

pub (crate) const CHANNEL_CAPACITY: usize = 1024;

/// List of commands that the Bakery system can handle.
#[derive(Debug)]
pub enum BakeryCommands {
    /// Clears all queues for an interface and removes all IP mappings from the XDP system.
    /// Use when replacing the entire hierarchy or at startup.
    ClearPriorSettings,

    /// Creates a new top-level MQ for a given interface, along with the default HTB hierarchy.
    MqSetup,
}

/// Starts the Bakery system, returning a channel sender for sending commands to the Bakery.
pub fn start_bakery() -> anyhow::Result<crossbeam_channel::Sender<BakeryCommands>> {
    let (tx, rx) = crossbeam_channel::bounded(CHANNEL_CAPACITY);
    std::thread::Builder::new()
        .name("lqos_bakery".to_string())
        .spawn(move || {
            bakery(rx);
        })
        .map_err(|e| anyhow::anyhow!("Failed to start Bakery thread: {}", e))?;
    Ok(tx)
}

fn bakery(rx: Receiver<BakeryCommands>) {
    while let Ok(command) = rx.recv() {
        if let Err(e) = match command {
            BakeryCommands::ClearPriorSettings => clear_prior_settings(),
            BakeryCommands::MqSetup => mq_setup(),
            _ => {
                unimplemented!("Bakery command not implemented: {:?}", command);
            }
        } {
            error!("Bakery command failed: {:?}, error: {}", command, e);
        }
    }
    error!("Bakery thread exited unexpectedly.");
}

fn clear_prior_settings() -> anyhow::Result<()> {
    let config = lqos_config::load_config()?;
    tc_control::clear_all_queues(&config.internet_interface())?;
    if !config.on_a_stick_mode() {
        tc_control::clear_all_queues(&config.isp_interface())?;
    }
    Ok(())
}

/// Calculate the appropriate r2q value based on the maximum bandwidth.
/// This matches Python's calculateR2q function.
fn calculate_r2q(max_rate_mbps: f64) -> u64 {
    const MAX_R2Q: u64 = 60_000; // See https://lartc.vger.kernel.narkive.com/NKaH1ZNG/htb-quantum-of-class-100001-is-small-consider-r2q-change
    let max_rate_bytes_per_second = max_rate_mbps * 125_000.0;
    let mut r2q = 10;
    
    // Use floating point division to match Python's behavior exactly
    while (max_rate_bytes_per_second / r2q as f64) > MAX_R2Q as f64 {
        r2q += 1;
    }
    r2q
}

fn queues_available_on_interface(interface: &str) -> anyhow::Result<usize> {
    let path = format!("/sys/class/net/{interface}/queues/");
    let sys_path = Path::new(&path);
    if !sys_path.exists() {
        error!(
            "/sys/class/net/{interface}/queues/ does not exist. Does this card only support one queue (not supported)?"
        );
        return Err(anyhow::anyhow!(
            "/sys/class/net/{interface}/queues/ does not exist. Does this card only support one queue (not supported)?"
        ));
    }

    let mut counts = (0, 0);
    let paths = std::fs::read_dir(sys_path)?;
    for path in paths {
        if let Ok(path) = &path {
            if path.path().is_dir() {
                if let Some(filename) = path.path().file_name() {
                    if let Some(filename) = filename.to_str() {
                        if filename.starts_with("rx-") {
                            counts.0 += 1;
                        } else if filename.starts_with("tx-") {
                            counts.1 += 1;
                        }
                    }
                }
            }
        }
    }

    Ok(usize::min(counts.0, counts.1))
}

fn queues_available() -> anyhow::Result<usize> {
    let config = lqos_config::load_config()?;
    let mut queues = 0;
    if config.on_a_stick_mode() {
        queues = queues_available_on_interface(&config.internet_interface())?;
        queues /= 2;
    } else {
        let internet_queues = queues_available_on_interface(&config.internet_interface())?;
        let isp_queues = queues_available_on_interface(&config.isp_interface())?;
        queues = usize::min(internet_queues, isp_queues);
    }

    Ok(queues)
}

fn mq_setup() -> anyhow::Result<()> {
    let config = lqos_config::load_config()?;

    // Calculations
    let downlink_mbps = config.queues.downlink_bandwidth_mbps as f64;
    let uplink_mbps = config.queues.uplink_bandwidth_mbps as f64;
    let max_bandwidth = f64::max(downlink_mbps, uplink_mbps);
    let r2q = calculate_r2q(max_bandwidth);
    let n_queues = queues_available()?;
    let sqm_chunks = config.queues.default_sqm.split(' ').collect::<Vec<&str>>();

    // Create the MQ discipline on the internet interface
    tc_control::replace_mq(&config.internet_interface())?;

    // Create the HTB hierarchy on the internet interface
    for queue in 0 .. n_queues {
        /*
        # MAKE TOP HTB
        command = 'qdisc add dev ' + thisInterface + ' parent 7FFF:' + hex(queue+1) + ' handle ' + hex(queue+1) + ': htb default 2'
        # MAKE PARENT CLASS
        command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+1) + ': classid ' + hex(queue+1) + ':1 htb rate '+ str(upstream_bandwidth_capacity_download_mbps()) + 'mbit ceil ' + str(upstream_bandwidth_capacity_download_mbps()) + 'mbit' + quantum(upstream_bandwidth_capacity_download_mbps())
        # MAKE DEFAULT SQM BUCKET
        command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':1 ' + sqm()
        # Default class - traffic gets passed through this limiter with lower priority if it enters the top HTB without a specific class.
        # Technically, that should not even happen. So don't expect much if any traffic in this default class.
        # Only 1/4 of defaultClassCapacity is guaranteed (to prevent hitting ceiling of upstream), for the most part it serves as an "up to" ceiling.
        command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':1 classid ' + hex(queue+1) + ':2 htb rate ' + str(round((upstream_bandwidth_capacity_download_mbps()-1)/4)) + 'mbit ceil ' + str(upstream_bandwidth_capacity_download_mbps()-1) + 'mbit prio 5' + quantum(upstream_bandwidth_capacity_download_mbps())
        command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':2 ' + sqm()
         */
        
        // Make top HTB (note that queue+1 is handled in the function)
        tc_control::make_top_htb(&config.internet_interface(), queue as u32,)?;

        // Make parent class
        tc_control::make_parent_class(&config.internet_interface(), queue as u32, downlink_mbps, r2q)?;

        // Make default SQM bucket
        tc_control::make_default_sqm_bucket(&config.internet_interface(), queue as u32, &sqm_chunks)?;

        // Make default class
        tc_control::make_default_class(&config.internet_interface(), queue as u32, downlink_mbps, r2q)?;

        // Make the CAKE queue for the default class
        tc_control::make_default_class_sqm(&config.internet_interface(), queue as u32, &sqm_chunks)?;
    }

    // Secondary interface setup
    /*
    thisInterface = interface_b()
    logging.info("# MQ Setup for " + thisInterface)
    if not on_a_stick():
        command = 'qdisc replace dev ' + thisInterface + ' root handle 7FFF: mq'
    for queue in range(queuesAvailable):
        command = 'qdisc add dev ' + thisInterface + ' parent 7FFF:' + hex(queue+stickOffset+1) + ' handle ' + hex(queue+stickOffset+1) + ': htb default 2'
        command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+stickOffset+1) + ': classid ' + hex(queue+stickOffset+1) + ':1 htb rate '+ str(upstream_bandwidth_capacity_upload_mbps()) + 'mbit ceil ' + str(upstream_bandwidth_capacity_upload_mbps()) + 'mbit' + quantum(upstream_bandwidth_capacity_upload_mbps())
        command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+stickOffset+1) + ':1 ' + sqm()
        # Default class - traffic gets passed through this limiter with lower priority if it enters the top HTB without a specific class.
        # Technically, that should not even happen. So don't expect much if any traffic in this default class.
        # Only 1/4 of defaultClassCapacity is guarenteed (to prevent hitting ceiling of upstream), for the most part it serves as an "up to" ceiling.
        command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+stickOffset+1) + ':1 classid ' + hex(queue+stickOffset+1) + ':2 htb rate ' + str(round((upstream_bandwidth_capacity_upload_mbps()-1)/4)) + 'mbit ceil ' + str(upstream_bandwidth_capacity_upload_mbps()-1) + 'mbit prio 5' + quantum(upstream_bandwidth_capacity_upload_mbps())
        command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+stickOffset+1) + ':2 ' + sqm()
    */
    let mut this_interface = config.isp_interface();
    let mut stick_offset = 0;
    if !config.on_a_stick_mode() {
        tc_control::replace_mq(&config.isp_interface())?;
        this_interface = config.internet_interface();
        stick_offset = queues_available()?; // The number of queues on the internet interface
    }

    for queue in 0 .. n_queues {
        // Make top HTB (note that queue+1 is handled in the function)
        tc_control::make_top_htb(&this_interface, queue as u32 + stick_offset as u32)?;

        // Make parent class
        tc_control::make_parent_class(&this_interface, queue as u32 + stick_offset as u32, uplink_mbps, r2q)?;

        // Make default SQM bucket
        tc_control::make_default_sqm_bucket(&this_interface, queue as u32 + stick_offset as u32, &sqm_chunks)?;

        // Make default class
        tc_control::make_default_class(&this_interface, queue as u32 + stick_offset as u32, uplink_mbps, r2q)?;

        // Make the CAKE queue for the default class
        tc_control::make_default_class_sqm(&this_interface, queue as u32 + stick_offset as u32, &sqm_chunks)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_r2q() {
        // Test values calculated from Python's calculateR2q function
        // Python: calculateR2q(1) -> R2Q = 10
        assert_eq!(calculate_r2q(1.0), 10);
        
        // Python: calculateR2q(10) -> R2Q = 21 
        assert_eq!(calculate_r2q(10.0), 21);
        
        // Python: calculateR2q(100) -> R2Q = 209
        assert_eq!(calculate_r2q(100.0), 209);
        
        // Python: calculateR2q(1000) -> R2Q = 2084
        assert_eq!(calculate_r2q(1000.0), 2084);
        
        // Python: calculateR2q(10000) -> R2Q = 20834
        assert_eq!(calculate_r2q(10000.0), 20834);
        
        // Test fractional values
        assert_eq!(calculate_r2q(1.5), 10);
        assert_eq!(calculate_r2q(999.9), 2084);
    }
}