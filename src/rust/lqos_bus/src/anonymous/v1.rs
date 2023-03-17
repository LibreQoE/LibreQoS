#[derive(Default, Debug)]

/// Defines data to be submitted if anonymous usage submission is
/// enabled. This is protocol version 1.
pub struct AnonymousUsageV1 {
    /// Total installed RAM (bytes)
    pub total_memory: u64,

    /// Total available RAM (bytes)
    pub available_memory: u64,

    /// Linux Kernel Version
    pub kernel_version: String,

    /// Number of "usable" CPU cores, as used by eBPF. This may not
    /// be exactly equal to the number of actual cores.
    pub usable_cores: u32,

    /// CPU brand
    pub cpu_brand: String,

    /// CPU vendor
    pub cpu_vendor: String,

    /// CPU frequency
    pub cpu_frequency: u64,

    /// Installed network cards
    pub nics: Vec<NicV1>,

    /// SQM setting from the ispConfig.py file
    pub sqm: String,

    /// Is Monitor-ony mode enabled?
    pub monitor_mode: bool,

    /// Capacity as specified in ispConfig.py
    pub total_capacity: (u32, u32),

    /// Generated node capacity from ispConfig.py
    pub generated_pdn_capacity: (u32, u32),

    /// Number of shaped devices from ShapedDevices.csv
    pub shaped_device_count: usize,

    /// Number of nodes read from network.json
    pub net_json_len: usize,
}


/// Description of installed NIC (version 1 data)
#[derive(Default, Debug)]
pub struct NicV1 {
    /// Description, usually "Ethernet"
    pub description: String,

    /// Product name as specified by the driver
    pub product: String,

    /// Vendor as specified by the driver
    pub vendor: String,

    /// Clock speed, specified by the vendor (may not be accurate)
    pub clock: String,

    /// NIC possible capacity (as reported by the driver)
    pub capacity: String,
}