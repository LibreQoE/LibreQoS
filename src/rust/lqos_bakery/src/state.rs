use std::collections::{HashMap, HashSet};

/// Information needed to create a circuit queue (Phase 2 lazy queues)
#[derive(Debug, Clone)]
pub struct CircuitQueueInfo {
    /// Network interface name (e.g., "eth0")
    pub interface: String,
    /// Parent class ID (e.g., "1:10")
    pub parent: String,
    /// Class ID for this circuit (e.g., "1:100")
    pub class_id: String,
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
    /*/// Pending circuit updates to batch and deduplicate
    pub pending_updates: HashSet<i64>,
    /// Pending circuit creates to batch and deduplicate
    pub pending_creates: HashSet<i64>,*/
}
