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

// Re-export commonly used TC control functions
pub use tc_control::{
    format_rate_for_tc,
    quantum,
    add_htb_class,
    add_circuit_htb_class,
    add_structural_htb_class,
    add_circuit_qdisc,
    sqm_fixup_rate,
};

use std::path::Path;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use crossbeam_channel::Receiver;
use tracing::{error, info, warn};

pub (crate) const CHANNEL_CAPACITY: usize = 1024;

/// Get current Unix timestamp in seconds
fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// List of commands that the Bakery system can handle.
#[derive(Debug)]
pub enum BakeryCommands {
    /// Clears all queues for an interface and removes all IP mappings from the XDP system.
    /// Use when replacing the entire hierarchy or at startup.
    ClearPriorSettings,

    /// Creates a new top-level MQ for a given interface, along with the default HTB hierarchy.
    MqSetup,

    /// Add an HTB class for a structural node (site/AP from network.json).
    /// These are intermediate nodes in the hierarchy, NOT leaf nodes.
    AddStructuralHTBClass {
        /// Network interface name (e.g., "eth0")
        interface: String,
        /// Parent class ID (e.g., "1:")
        parent: String,
        /// Class ID for this node (e.g., "1:10")
        classid: String,
        /// Minimum bandwidth in Mbps
        rate_mbps: f64,
        /// Maximum bandwidth in Mbps
        ceil_mbps: f64,
        /// Hash of site name for tracking
        site_hash: i64,
        /// R2Q value for quantum calculation
        r2q: u64,
    },

    /// Add an HTB class for a circuit (customer circuit from ShapedDevices.csv).
    /// These are leaf nodes that shape actual customer traffic.
    AddCircuitHTBClass {
        /// Network interface name (e.g., "eth0")
        interface: String,
        /// Parent class ID (e.g., "1:10")
        parent: String,
        /// Class ID for this circuit (e.g., "1:100")
        classid: String,
        /// Minimum bandwidth in Mbps
        rate_mbps: f64,
        /// Maximum bandwidth in Mbps
        ceil_mbps: f64,
        /// Hash of circuit ID for tracking
        circuit_hash: i64,
        /// Optional comment for debugging
        comment: Option<String>,
        /// R2Q value for quantum calculation
        r2q: u64,
    },

    /// Add a qdisc (CAKE/fq_codel) to a circuit class.
    /// This is ONLY for circuits (leaf nodes). Structural nodes do NOT get qdiscs.
    AddCircuitQdisc {
        /// Network interface name (e.g., "eth0")
        interface: String,
        /// Major part of parent class ID
        parent_major: u32,
        /// Minor part of parent class ID
        parent_minor: u32,
        /// Hash of circuit ID for tracking
        circuit_hash: i64,
        /// SQM parameters (split from sqm string)
        sqm_params: Vec<String>,
    },

    /// Execute a batch of TC commands (alternative bulk approach).
    /// Write commands to file and execute with `tc -b` like Python does.
    ExecuteTCCommands {
        /// Vector of TC command strings (without /sbin/tc prefix)
        commands: Vec<String>,
        /// Whether to use -f flag to force execution
        force_mode: bool,
    },

    /// Update circuit last activity timestamp (Phase 2 lazy queues)
    UpdateCircuit {
        /// Hash of circuit ID to update
        circuit_hash: i64,
    },

    /// Create circuit queue if not already created (Phase 2 lazy queues)
    CreateCircuit {
        /// Hash of circuit ID to create
        circuit_hash: i64,
    },
}

/// Information needed to create a circuit queue (Phase 2 lazy queues)
#[derive(Debug, Clone)]
pub struct CircuitQueueInfo {
    /// Network interface name (e.g., "eth0")
    pub interface: String,
    /// Parent class ID (e.g., "1:10")
    pub parent: String,
    /// Class ID for this circuit (e.g., "1:100")
    pub classid: String,
    /// Minimum bandwidth in Mbps
    pub rate_mbps: f64,
    /// Maximum bandwidth in Mbps
    pub ceil_mbps: f64,
    /// Hash of circuit ID for tracking
    pub circuit_hash: i64,
    /// Optional comment for debugging
    pub comment: Option<String>,
    /// R2Q value for quantum calculation
    pub r2q: u64,
    /// SQM parameters for the qdisc
    pub sqm_params: Vec<String>,
    /// Whether the queue has been actually created in TC
    pub created: bool,
    /// Last updated timestamp (unix time)
    pub last_updated: u64,
}

/// Information about a structural queue node (Phase 2 lazy queues)
#[derive(Debug, Clone)]
pub struct StructuralQueueInfo {
    /// Network interface name (e.g., "eth0")
    pub interface: String,
    /// Parent class ID (e.g., "1:")
    pub parent: String,
    /// Class ID for this node (e.g., "1:10")
    pub classid: String,
    /// Minimum bandwidth in Mbps
    pub rate_mbps: f64,
    /// Maximum bandwidth in Mbps
    pub ceil_mbps: f64,
    /// Hash of site name for tracking
    pub site_hash: i64,
    /// R2Q value for quantum calculation
    pub r2q: u64,
}

/// Shared state for the Bakery system (Phase 2 lazy queues)
#[derive(Debug, Default)]
pub struct BakeryState {
    /// Storage for circuit queue information, indexed by circuit_hash
    pub circuits: HashMap<i64, CircuitQueueInfo>,
    /// Storage for structural queue information, indexed by site_hash
    pub structural: HashMap<i64, StructuralQueueInfo>,
    /// Pending circuit updates to batch and deduplicate
    pub pending_updates: HashSet<i64>,
    /// Pending circuit creates to batch and deduplicate
    pub pending_creates: HashSet<i64>,
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
    // Initialize shared state for Phase 2 lazy queues
    let state = Arc::new(Mutex::new(BakeryState::default()));
    
    // Clear the TC output file when bakery starts (if in file mode)
    #[cfg(not(test))]
    if crate::tc_control::is_write_to_file_mode() {
        if let Ok(config) = lqos_config::load_config() {
            let output_path = std::path::Path::new(&config.lqos_directory).join("tc-rust.txt");
            if output_path.exists() {
                if let Err(e) = std::fs::remove_file(&output_path) {
                    warn!("Failed to remove old TC output file: {}", e);
                } else {
                    info!("Cleared old TC output file: {:?}", output_path);
                }
            }
        }
    }
    
    while let Ok(command) = rx.recv() {
        info!("🍞 Bakery received command: {:?}", command);
        if let Err(e) = match &command {
            BakeryCommands::ClearPriorSettings => clear_prior_settings(),
            BakeryCommands::MqSetup => mq_setup(),
            
            BakeryCommands::AddStructuralHTBClass { 
                interface, parent, classid, rate_mbps, ceil_mbps, site_hash, r2q 
            } => {
                handle_add_structural_htb_class(
                    &state, interface, parent, classid, *rate_mbps, *ceil_mbps, *site_hash, *r2q
                )
            },
            
            BakeryCommands::AddCircuitHTBClass { 
                interface, parent, classid, rate_mbps, ceil_mbps, circuit_hash, comment, r2q 
            } => {
                handle_add_circuit_htb_class(
                    &state, interface, parent, classid, *rate_mbps, *ceil_mbps, 
                    *circuit_hash, comment.clone(), *r2q
                )
            },
            
            BakeryCommands::AddCircuitQdisc { 
                interface, parent_major, parent_minor, circuit_hash, sqm_params 
            } => {
                handle_add_circuit_qdisc(
                    &state, interface, *parent_major, *parent_minor, *circuit_hash, sqm_params.clone()
                )
            },
            
            BakeryCommands::UpdateCircuit { circuit_hash } => {
                handle_update_circuit(&state, *circuit_hash)
            },
            
            BakeryCommands::CreateCircuit { circuit_hash } => {
                handle_create_circuit(&state, *circuit_hash)
            },
            
            BakeryCommands::ExecuteTCCommands { commands, force_mode } => {
                execute_tc_commands_bulk(&state, commands.clone(), *force_mode)
            },
        } {
            error!("Bakery command failed: {:?}, error: {}", command, e);
        }
    }
    error!("Bakery thread exited unexpectedly.");
}

/// Check if lazy queues are enabled in configuration
fn is_lazy_queues_enabled() -> bool {
    if let Ok(config) = lqos_config::load_config() {
        let lazy_enabled = config.queues.lazy_queues.unwrap_or(false);
        info!("Lazy queues configuration check: lazy_queues = {:?}, enabled = {}", 
              config.queues.lazy_queues, lazy_enabled);
        lazy_enabled
    } else {
        warn!("Failed to load config for lazy queues check");
        false
    }
}

/// Handle AddStructuralHTBClass command (Phase A: Structural Queues First)
fn handle_add_structural_htb_class(
    state: &Arc<Mutex<BakeryState>>,
    interface: &str,
    parent: &str,
    classid: &str,
    rate_mbps: f64,
    ceil_mbps: f64,
    site_hash: i64,
    r2q: u64,
) -> anyhow::Result<()> {
    // Always create structural queues immediately AND store them
    tc_control::add_structural_htb_class(
        interface, parent, classid, rate_mbps, ceil_mbps, site_hash, r2q
    )?;
    
    // Store structural queue info for tracking
    let mut state_lock = state.lock().map_err(|_| anyhow::anyhow!("Failed to acquire state lock"))?;
    let structural_info = StructuralQueueInfo {
        interface: interface.to_string(),
        parent: parent.to_string(),
        classid: classid.to_string(),
        rate_mbps,
        ceil_mbps,
        site_hash,
        r2q,
    };
    state_lock.structural.insert(site_hash, structural_info);
    
    info!("Created structural HTB class for site_hash {}: {}", site_hash, classid);
    Ok(())
}

/// Handle AddCircuitHTBClass command (Phase B: Circuit Storage)
fn handle_add_circuit_htb_class(
    state: &Arc<Mutex<BakeryState>>,
    interface: &str,
    parent: &str,
    classid: &str,
    rate_mbps: f64,
    ceil_mbps: f64,
    circuit_hash: i64,
    comment: Option<String>,
    r2q: u64,
) -> anyhow::Result<()> {
    if is_lazy_queues_enabled() {
        // Phase 2: Store circuit info but don't create queue yet
        let mut state_lock = state.lock().map_err(|_| anyhow::anyhow!("Failed to acquire state lock"))?;
        
        // Update existing circuit info or create new one
        if let Some(circuit_info) = state_lock.circuits.get_mut(&circuit_hash) {
            // Update existing circuit info
            circuit_info.interface = interface.to_string();
            circuit_info.parent = parent.to_string();
            circuit_info.classid = classid.to_string();
            circuit_info.rate_mbps = rate_mbps;
            circuit_info.ceil_mbps = ceil_mbps;
            circuit_info.comment = comment;
            circuit_info.r2q = r2q;
        } else {
            // Create new circuit info
            let circuit_info = CircuitQueueInfo {
                interface: interface.to_string(),
                parent: parent.to_string(),
                classid: classid.to_string(),
                rate_mbps,
                ceil_mbps,
                circuit_hash,
                comment,
                r2q,
                sqm_params: Vec::new(), // Will be set by AddCircuitQdisc
                created: false,
                last_updated: 0,
            };
            state_lock.circuits.insert(circuit_hash, circuit_info);
        }
        
        info!("Stored circuit HTB class info for circuit_hash {}: {} (lazy creation)", circuit_hash, classid);
    } else {
        // Phase 1: Create circuit queue immediately (backward compatibility)
        tc_control::add_circuit_htb_class(
            interface, parent, classid, rate_mbps, ceil_mbps, circuit_hash, 
            comment.as_deref(), r2q
        )?;
        info!("Created circuit HTB class immediately for circuit_hash {}: {}", circuit_hash, classid);
    }
    
    Ok(())
}

/// Handle AddCircuitQdisc command (Phase B: Circuit Storage)
fn handle_add_circuit_qdisc(
    state: &Arc<Mutex<BakeryState>>,
    interface: &str,
    parent_major: u32,
    parent_minor: u32,
    circuit_hash: i64,
    sqm_params: Vec<String>,
) -> anyhow::Result<()> {
    if is_lazy_queues_enabled() {
        // Phase 2: Update circuit info with SQM parameters but don't create qdisc yet
        let mut state_lock = state.lock().map_err(|_| anyhow::anyhow!("Failed to acquire state lock"))?;
        
        if let Some(circuit_info) = state_lock.circuits.get_mut(&circuit_hash) {
            circuit_info.sqm_params = sqm_params;
            info!("Updated circuit qdisc info for circuit_hash {}: stored SQM params (lazy creation)", circuit_hash);
        } else {
            warn!("Circuit qdisc command for unknown circuit_hash {}", circuit_hash);
        }
    } else {
        // Phase 1: Create qdisc immediately (backward compatibility)
        let sqm_strs: Vec<&str> = sqm_params.iter().map(|s| s.as_str()).collect();
        tc_control::add_circuit_qdisc(
            interface, parent_major, parent_minor, circuit_hash, &sqm_strs
        )?;
        info!("Created circuit qdisc immediately for circuit_hash {}", circuit_hash);
    }
    
    Ok(())
}

/// Handle UpdateCircuit command (Phase C: Lazy Creation - Update)
fn handle_update_circuit(
    state: &Arc<Mutex<BakeryState>>,
    circuit_hash: i64,
) -> anyhow::Result<()> {
    if !is_lazy_queues_enabled() {
        // Lazy queues disabled, ignore update commands
        return Ok(());
    }
    
    let mut state_lock = state.lock().map_err(|_| anyhow::anyhow!("Failed to acquire state lock"))?;
    
    if let Some(circuit_info) = state_lock.circuits.get_mut(&circuit_hash) {
        circuit_info.last_updated = current_timestamp();
        
        // If not created yet, create it now
        if !circuit_info.created {
            drop(state_lock); // Release lock before creating queue
            handle_create_circuit(state, circuit_hash)?;
        }
    } else {
        warn!("Update requested for unknown circuit_hash {}", circuit_hash);
    }
    
    Ok(())
}

/// Handle CreateCircuit command (Phase C: Lazy Creation - Create)
fn handle_create_circuit(
    state: &Arc<Mutex<BakeryState>>,
    circuit_hash: i64,
) -> anyhow::Result<()> {
    if !is_lazy_queues_enabled() {
        // Lazy queues disabled, ignore create commands
        return Ok(());
    }
    
    let mut state_lock = state.lock().map_err(|_| anyhow::anyhow!("Failed to acquire state lock"))?;
    
    if let Some(circuit_info) = state_lock.circuits.get_mut(&circuit_hash) {
        // Check if already created (prevent duplicates)
        if circuit_info.created {
            return Ok(());
        }
        
        // Clone data needed for creation (to release lock early)
        let interface = circuit_info.interface.clone();
        let parent = circuit_info.parent.clone();
        let classid = circuit_info.classid.clone();
        let rate_mbps = circuit_info.rate_mbps;
        let ceil_mbps = circuit_info.ceil_mbps;
        let comment = circuit_info.comment.clone();
        let r2q = circuit_info.r2q;
        let sqm_params = circuit_info.sqm_params.clone();
        
        // Mark as created before releasing lock
        circuit_info.created = true;
        circuit_info.last_updated = current_timestamp();
        
        // Release lock before executing TC commands
        drop(state_lock);
        
        // Create HTB class
        tc_control::add_circuit_htb_class(
            &interface, &parent, &classid, rate_mbps, ceil_mbps, circuit_hash, 
            comment.as_deref(), r2q
        )?;
        
        // Create qdisc if SQM params are available
        if !sqm_params.is_empty() {
            // Parse classid to get parent_major and parent_minor
            if let Some(colon_pos) = classid.find(':') {
                if let (Ok(major), Ok(minor)) = (
                    classid[..colon_pos].parse::<u32>(),
                    classid[colon_pos + 1..].parse::<u32>(),
                ) {
                    let sqm_strs: Vec<&str> = sqm_params.iter().map(|s| s.as_str()).collect();
                    tc_control::add_circuit_qdisc(
                        &interface, major, minor, circuit_hash, &sqm_strs
                    )?;
                }
            }
        }
        
        info!("Created circuit queue for circuit_hash {}: {}", circuit_hash, classid);
    } else {
        warn!("Create requested for unknown circuit_hash {}", circuit_hash);
    }
    
    Ok(())
}

fn clear_prior_settings() -> anyhow::Result<()> {
    let config = lqos_config::load_config()?;
    
    // Check if MQ is installed (Python checks for 'mq' in output)
    if tc_control::has_mq_qdisc(&config.internet_interface())? {
        info!("MQ detected. Will delete and recreate mq qdisc.");
        
        // Clear TC on interface A
        tc_control::delete_root_qdisc(&config.internet_interface())?;
        
        // Clear TC on interface B if not on-a-stick mode
        if !config.on_a_stick_mode() {
            tc_control::delete_root_qdisc(&config.isp_interface())?;
        }
    }
    
    // Note: Python also clears IP mappings here, but that's handled elsewhere in Rust
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
    let mut queues;
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

/// Execute a batch of TC commands using tc -b (bulk mode) like Python does
/// 
/// # Arguments
/// * `commands` - Vec of TC command strings (without /sbin/tc prefix)
/// * `force_mode` - Whether to use -f flag to force execution and ignore errors
/// 
/// # Returns  
/// * `Result<(), anyhow::Error>` - Returns Ok if successful, or an error if execution fails
fn execute_tc_commands_bulk(
    state: &Arc<Mutex<BakeryState>>,
    commands: Vec<String>, 
    force_mode: bool
) -> anyhow::Result<()> {
    info!("🍞 Processing {} TC commands in bulk mode", commands.len());
    
    if is_lazy_queues_enabled() {
        // NEW APPROACH: Parse commands and separate structural from circuit
        // LibreQoS.py needs to include circuit_hash in comments for this to work
        parse_and_route_tc_commands(state, commands, force_mode)
    } else {
        // Execute all commands immediately (Phase 1 behavior)
        execute_tc_commands_immediate(commands, force_mode)
    }
}

/// Parse TC commands and route circuit commands through lazy queue logic
fn parse_and_route_tc_commands(
    state: &Arc<Mutex<BakeryState>>,
    commands: Vec<String>,
    force_mode: bool,
) -> anyhow::Result<()> {
    let mut structural_commands = Vec::new();
    let mut deferred_count = 0;
    
    // First pass: Execute all structural commands and defer circuit commands
    for command in &commands {
        if let Some((is_circuit, circuit_hash)) = parse_tc_command_type(command) {
            if is_circuit {
                // This is a circuit command - defer for lazy creation
                if let Some(hash) = circuit_hash {
                    store_circuit_command(state, command, hash)?;
                    deferred_count += 1;
                } else {
                    warn!("Circuit command without hash, executing immediately: {}", command);
                    structural_commands.push(command.clone());
                }
            } else {
                // This is a structural command - execute immediately
                structural_commands.push(command.clone());
            }
        } else {
            // Unknown command type - execute immediately for safety
            structural_commands.push(command.clone());
        }
    }
    
    // Execute all structural commands first to build the hierarchy
    if !structural_commands.is_empty() {
        info!("⚡ Executing {} structural/other commands immediately", structural_commands.len());
        execute_tc_commands_immediate(structural_commands, force_mode)?;
    }
    
    info!("✅ Bulk command processing complete: {} circuit commands deferred for lazy creation", 
          deferred_count);
    Ok(())
}

/// Determine if a TC command is for a circuit (vs structural) and extract circuit_hash if present
fn parse_tc_command_type(command: &str) -> Option<(bool, Option<i64>)> {
    // Look for circuit_hash in comment: "# circuit_hash: 1234567890"
    if let Some(hash_pos) = command.find("# circuit_hash:") {
        let hash_str = &command[hash_pos + 15..];
        if let Some(end_pos) = hash_str.find(' ').or_else(|| Some(hash_str.len())) {
            if let Ok(hash) = hash_str[..end_pos].trim().parse::<i64>() {
                return Some((true, Some(hash)));
            }
        }
    }
    
    // Check if this looks like a circuit command based on structure
    let parts: Vec<&str> = command.split_whitespace().collect();
    if parts.len() >= 2 {
        match (parts[0], parts[1]) {
            ("class", "add") => {
                // Check for rate/ceil to distinguish circuit from structural
                if command.contains(" rate ") && command.contains(" ceil ") {
                    // Look at the rate to determine if it's likely a circuit
                    if let Some(rate_mbps) = extract_rate_mbps(command) {
                        // Heuristic: Circuit classes typically have rates < 1000 Mbps
                        return Some((rate_mbps < 1000.0, None));
                    }
                }
            },
            ("qdisc", "add") => {
                // CAKE/fq_codel qdiscs are typically for circuits
                if command.contains(" cake ") || command.contains(" fq_codel ") {
                    return Some((true, None));
                }
            },
            _ => {}
        }
    }
    
    // Default: treat as structural
    Some((false, None))
}

/// Extract rate in Mbps from a TC command
fn extract_rate_mbps(command: &str) -> Option<f64> {
    if let Some(rate_pos) = command.find(" rate ") {
        let rate_str = &command[rate_pos + 6..];
        if let Some(space_pos) = rate_str.find(' ') {
            let rate_value = &rate_str[..space_pos];
            return parse_tc_rate_to_mbps(rate_value);
        }
    }
    None
}

/// Parse TC rate strings like "500kbit", "45mbit", "1gbit" to Mbps
fn parse_tc_rate_to_mbps(rate_str: &str) -> Option<f64> {
    if rate_str.ends_with("kbit") {
        let value: f64 = rate_str.trim_end_matches("kbit").parse().ok()?;
        Some(value / 1000.0)
    } else if rate_str.ends_with("mbit") {
        let value: f64 = rate_str.trim_end_matches("mbit").parse().ok()?;
        Some(value)
    } else if rate_str.ends_with("gbit") {
        let value: f64 = rate_str.trim_end_matches("gbit").parse().ok()?;
        Some(value * 1000.0)
    } else {
        None
    }
}

/// Store a circuit command for later lazy execution
fn store_circuit_command(
    state: &Arc<Mutex<BakeryState>>,
    command: &str,
    circuit_hash: i64,
) -> anyhow::Result<()> {
    // Parse the command to extract parameters
    let parts: Vec<&str> = command.split_whitespace().collect();
    
    match (parts.get(0), parts.get(1)) {
        (Some(&"class"), Some(&"add")) => {
            // Parse HTB class parameters
            if let Some(params) = parse_htb_class_params(&parts) {
                handle_add_circuit_htb_class(
                    state,
                    &params.interface,
                    &params.parent,
                    &params.classid,
                    params.rate_mbps,
                    params.ceil_mbps,
                    circuit_hash,
                    None,
                    params.quantum.unwrap_or(10), // Default r2q
                )?;
                info!("📝 Stored circuit HTB class for lazy creation: {} (hash: {})", 
                      params.classid, circuit_hash);
            }
        },
        (Some(&"qdisc"), Some(&"add")) => {
            // Parse qdisc parameters
            if let Some(params) = parse_qdisc_params(&parts) {
                handle_add_circuit_qdisc(
                    state,
                    &params.interface,
                    params.parent_major,
                    params.parent_minor,
                    circuit_hash,
                    params.sqm_params,
                )?;
                info!("📝 Stored circuit qdisc for lazy creation: {}:{} (hash: {})", 
                      params.parent_major, params.parent_minor, circuit_hash);
            }
        },
        _ => {
            warn!("Unknown circuit command type: {}", command);
        }
    }
    
    Ok(())
}

struct HtbClassParams {
    interface: String,
    parent: String,
    classid: String,
    rate_mbps: f64,
    ceil_mbps: f64,
    quantum: Option<u64>,
}

fn parse_htb_class_params(parts: &[&str]) -> Option<HtbClassParams> {
    let mut interface = None;
    let mut parent = None;
    let mut classid = None;
    let mut rate = None;
    let mut ceil = None;
    let mut quantum = None;
    
    let mut i = 0;
    while i < parts.len() - 1 {
        match parts[i] {
            "dev" => interface = Some(parts[i + 1].to_string()),
            "parent" => parent = Some(parts[i + 1].to_string()),
            "classid" => classid = Some(parts[i + 1].to_string()),
            "rate" => rate = parse_tc_rate_to_mbps(parts[i + 1]),
            "ceil" => ceil = parse_tc_rate_to_mbps(parts[i + 1]),
            "quantum" => quantum = parts[i + 1].parse().ok(),
            _ => {}
        }
        i += 1;
    }
    
    Some(HtbClassParams {
        interface: interface?,
        parent: parent?,
        classid: classid?,
        rate_mbps: rate?,
        ceil_mbps: ceil?,
        quantum,
    })
}

struct QdiscParams {
    interface: String,
    parent_major: u32,
    parent_minor: u32,
    sqm_params: Vec<String>,
}

fn parse_qdisc_params(parts: &[&str]) -> Option<QdiscParams> {
    let mut interface = None;
    let mut parent = None;
    let mut sqm_start = None;
    
    let mut i = 0;
    while i < parts.len() - 1 {
        match parts[i] {
            "dev" => interface = Some(parts[i + 1].to_string()),
            "parent" => parent = Some(parts[i + 1]),
            "cake" | "fq_codel" => {
                sqm_start = Some(i);
                break;
            }
            _ => {}
        }
        i += 1;
    }
    
    let parent_str = parent?;
    let (parent_major, parent_minor) = parse_tc_handle(parent_str)?;
    
    let sqm_params = if let Some(start) = sqm_start {
        parts[start..].iter().map(|s| s.to_string()).collect()
    } else {
        Vec::new()
    };
    
    Some(QdiscParams {
        interface: interface?,
        parent_major,
        parent_minor,
        sqm_params,
    })
}

/// Parse TC handle strings like "1:100" or "0x1:0x64" to (major, minor)
fn parse_tc_handle(handle: &str) -> Option<(u32, u32)> {
    if let Some(colon_pos) = handle.find(':') {
        let major_str = &handle[..colon_pos];
        let minor_str = &handle[colon_pos + 1..];
        
        let major = if major_str.starts_with("0x") {
            u32::from_str_radix(&major_str[2..], 16).ok()?
        } else {
            major_str.parse().ok()?
        };
        
        let minor = if minor_str.starts_with("0x") {
            u32::from_str_radix(&minor_str[2..], 16).ok()?
        } else {
            minor_str.parse().ok()?
        };
        
        Some((major, minor))
    } else {
        None
    }
}

/// Execute TC commands immediately (Phase 1 behavior)
fn execute_tc_commands_immediate(commands: Vec<String>, force_mode: bool) -> anyhow::Result<()> {
    info!("⚡ Executing {} TC commands immediately", commands.len());
    
    // If we're in write-to-file mode, just write all commands using our centralized function
    if tc_control::is_write_to_file_mode() {
        for command in &commands {
            let args: Vec<&str> = command.split_whitespace().collect();
            tc_control::execute_tc_command(&args)?;
        }
        info!("Wrote {} TC commands to file", commands.len());
        return Ok(());
    }
    
    // Otherwise, execute using tc -b (bulk mode)
    use std::fs::File;
    use std::io::Write;
    
    const TC_BULK_FILE: &str = "tc-bulk-rust.txt";
    
    // Write all commands to a temporary file
    {
        let mut file = File::create(TC_BULK_FILE)?;
        for command in &commands {
            writeln!(file, "{}", command)?;
        }
    }
    
    // Execute using tc -b (bulk mode)
    let mut tc_command = std::process::Command::new("/sbin/tc");
    
    if force_mode {
        tc_command.arg("-f"); // Force mode - ignore errors
    }
    
    tc_command.arg("-b").arg(TC_BULK_FILE);
    
    let output = tc_command.output()?;
    
    // Clean up the temporary file
    let _ = std::fs::remove_file(TC_BULK_FILE);
    
    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "TC bulk command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    
    info!("Successfully executed {} TC commands in bulk mode", commands.len());
    Ok(())
}

// REMOVED: All TC command parsing code
// This approach was fundamentally flawed because:
// 1. circuit_hash must be derived from circuit ID in ShapedDevices.csv
// 2. TC commands only contain classids, not original circuit IDs
// 3. Cannot reverse-engineer circuit ID from classid
//
// The correct approach is to use individual BakeryCommands which already
// have the proper circuit_hash passed from LibreQoS.py

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
    
    #[test]
    fn test_bakery_commands_creation() {
        // Test that we can create all the new BakeryCommands variants
        
        let structural_cmd = BakeryCommands::AddStructuralHTBClass {
            interface: "eth0".to_string(),
            parent: "1:".to_string(),
            classid: "1:10".to_string(),
            rate_mbps: 100.0,
            ceil_mbps: 200.0,
            site_hash: 987654321,
            r2q: 21,
        };
        
        let circuit_cmd = BakeryCommands::AddCircuitHTBClass {
            interface: "eth0".to_string(),
            parent: "1:10".to_string(),
            classid: "1:100".to_string(),
            rate_mbps: 10.5,
            ceil_mbps: 15.0,
            circuit_hash: 1234567890,
            comment: Some("Customer ABC".to_string()),
            r2q: 21,
        };
        
        let qdisc_cmd = BakeryCommands::AddCircuitQdisc {
            interface: "eth0".to_string(),
            parent_major: 1,
            parent_minor: 100,
            circuit_hash: 1234567890,
            sqm_params: vec!["cake".to_string(), "bandwidth".to_string(), "15mbit".to_string()],
        };
        
        let bulk_cmd = BakeryCommands::ExecuteTCCommands {
            commands: vec!["class add dev eth0 parent 1: classid 1:1 htb rate 1000mbit".to_string()],
            force_mode: false,
        };
        
        // Verify we can format the commands for debugging
        assert!(format!("{:?}", structural_cmd).contains("AddStructuralHTBClass"));
        assert!(format!("{:?}", circuit_cmd).contains("AddCircuitHTBClass"));
        assert!(format!("{:?}", qdisc_cmd).contains("AddCircuitQdisc"));
        assert!(format!("{:?}", bulk_cmd).contains("ExecuteTCCommands"));
    }
    
    #[test]
    fn test_phase2_data_structures() {
        // Test CircuitQueueInfo creation
        let circuit_info = CircuitQueueInfo {
            interface: "eth0".to_string(),
            parent: "1:10".to_string(),
            classid: "1:100".to_string(),
            rate_mbps: 10.5,
            ceil_mbps: 15.0,
            circuit_hash: 1234567890,
            comment: Some("Test Circuit".to_string()),
            r2q: 21,
            sqm_params: vec!["cake".to_string(), "bandwidth".to_string(), "15mbit".to_string()],
            created: false,
            last_updated: 0,
        };
        
        assert_eq!(circuit_info.circuit_hash, 1234567890);
        assert_eq!(circuit_info.rate_mbps, 10.5);
        assert!(!circuit_info.created);
        
        // Test StructuralQueueInfo creation
        let structural_info = StructuralQueueInfo {
            interface: "eth0".to_string(),
            parent: "1:".to_string(),
            classid: "1:10".to_string(),
            rate_mbps: 100.0,
            ceil_mbps: 200.0,
            site_hash: 987654321,
            r2q: 21,
        };
        
        assert_eq!(structural_info.site_hash, 987654321);
        assert_eq!(structural_info.rate_mbps, 100.0);
        
        // Test BakeryState creation
        let state = BakeryState::default();
        assert!(state.circuits.is_empty());
        assert!(state.structural.is_empty());
        assert!(state.pending_updates.is_empty());
        assert!(state.pending_creates.is_empty());
    }
    
    #[test] 
    fn test_phase2_commands() {
        let update_cmd = BakeryCommands::UpdateCircuit {
            circuit_hash: 1234567890,
        };
        
        let create_cmd = BakeryCommands::CreateCircuit {
            circuit_hash: 1234567890,
        };
        
        // Verify we can format the new commands for debugging
        assert!(format!("{:?}", update_cmd).contains("UpdateCircuit"));
        assert!(format!("{:?}", create_cmd).contains("CreateCircuit"));
        assert!(format!("{:?}", update_cmd).contains("1234567890"));
    }
    
    #[test]
    fn test_current_timestamp() {
        let ts1 = current_timestamp();
        std::thread::sleep(std::time::Duration::from_millis(10));
        let ts2 = current_timestamp();
        
        // Timestamp should advance
        assert!(ts2 >= ts1);
        
        // Should be reasonable Unix timestamp (after year 2020)
        assert!(ts1 > 1_600_000_000);
    }
}