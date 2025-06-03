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

    /// Create a circuit queue if not already created (Phase 2 lazy queues)
    CreateCircuit {
        /// Hash of circuit ID to create
        circuit_hash: i64,
    },
}