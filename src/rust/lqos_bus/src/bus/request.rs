// SPDX-FileCopyrightText: 2025 LibreQoE support@libreqos.io
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-LibreQoS-Exception

use crate::TcHandle;
use allocative::Allocative;
use lqos_config::Tunables;
use serde::{Deserialize, Serialize};

/// Source system for an urgent issue
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Copy, Allocative)]
pub enum UrgentSource {
    /// Raised by the scheduler process
    Scheduler,
    /// Raised by the LibreQoS Python orchestrator
    LibreQoS,
    /// Raised by the local API server
    API,
    /// Raised by lqosd or other components
    System,
}

/// Severity of an urgent issue
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Copy, Allocative)]
pub enum UrgentSeverity {
    /// Error requires attention
    Error,
    /// Warning is informative/high-visibility
    Warning,
}

/// One or more `BusRequest` objects must be included in a `BusSession`
/// request. Each `BusRequest` represents a single request for action
/// or data.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Allocative)]
pub enum BusRequest {
    /// A generic "is it alive?" test. Returns an `Ack`.
    Ping,

    /// Request total current throughput. Returns a
    /// `BusResponse::CurrentThroughput` value.
    GetCurrentThroughput,

    /// Retrieve the top N downloads by bandwidth use.
    GetTopNDownloaders {
        /// First row to retrieve (usually 0 unless you are paging)
        start: u32,
        /// Last row to retrieve (10 for top-10 starting at 0)
        end: u32,
    },

    /// Retrieve the top N uploads by bandwidth use.
    GetTopNUploaders {
        /// First row to retrieve (usually 0 unless you are paging)
        start: u32,
        /// Last row to retrieve (10 for top-10 starting at 0)
        end: u32,
    },

    /// Retrieve per-circuit TemporalHeatmap blocks.
    GetCircuitHeatmaps,

    /// Retrieve per-site TemporalHeatmap blocks.
    GetSiteHeatmaps,

    /// Retrieve per-ASN TemporalHeatmap blocks.
    GetAsnHeatmaps,

    /// Retrieve the global (roll-up) TemporalHeatmap.
    GetGlobalHeatmap,

    /// Retrieve headline metrics for the Executive Summary tab.
    GetExecutiveSummaryHeader,

    /// Retrieves the TopN hosts with the worst RTT, sorted by RTT descending.
    GetWorstRtt {
        /// First row to retrieve (usually 0 unless you are paging)
        start: u32,
        /// Last row to retrieve (10 for top-10 starting at 0)
        end: u32,
    },

    /// Retrieves the TopN hosts with the worst Retransmits, sorted by Retransmits descending.
    GetWorstRetransmits {
        /// First row to retrieve (usually 0 unless you are paging)
        start: u32,
        /// Last row to retrieve (10 for top-10 starting at 0)
        end: u32,
    },

    /// Retrieves the TopN hosts with the best RTT, sorted by RTT descending.
    GetBestRtt {
        /// First row to retrieve (usually 0 unless you are paging)
        start: u32,
        /// Last row to retrieve (10 for top-10 starting at 0)
        end: u32,
    },

    /// Retrieves current byte counters for all hosts.
    GetHostCounter,

    /// Requests that the XDP back-end associate an IP address with a
    /// TC (traffic control) handle, and CPU. The "upload" flag indicates
    /// that this is a second channel applied to the SAME network interface,
    /// used for "on-a-stick" mode upload channels.
    MapIpToFlow {
        /// The IP address to map, as a string. It can be IPv4 or IPv6,
        /// and supports CIDR notation for subnets. "192.168.1.1",
        /// "192.168.1.0/24", are both valid.
        ip_address: String,

        /// The TC Handle to which the IP address should be mapped.
        tc_handle: TcHandle,

        /// The CPU on which the TC handle should be shaped.
        cpu: u32,

        /// Hashed circuit identifier (from ShapedDevices.csv).
        ///
        /// Defaults to `0` for backward compatibility.
        #[serde(default)]
        circuit_id: u64,

        /// Hashed device identifier (from ShapedDevices.csv).
        ///
        /// Defaults to `0` for backward compatibility.
        #[serde(default)]
        device_id: u64,

        /// If true, this is a *second* flow for the same IP range on
        /// the same NIC. Used for handling "on a stick" configurations.
        upload: bool,
    },

    /// After a batch of `MapIpToFlow` requests, this command will
    /// clear the hot cache, forcing the XDP program to re-read the
    /// mapping table.
    ClearHotCache,

    /// Requests that the XDP program unmap an IP address/subnet from
    /// the traffic management system.
    DelIpFlow {
        /// The IP address to unmap. It can be an IPv4, IPv6 or CIDR
        /// subnet.
        ip_address: String,

        /// Should we delete a secondary mapping (for upload)?
        upload: bool,
    },

    /// Clear all XDP IP/TC/CPU mappings.
    ClearIpFlow,

    /// Retreieve list of all current IP/TC/CPU mappings.
    ListIpFlow,

    /// Simulate the previous version's `xdp_pping` command, returning
    /// RTT data for all mapped flows by TC handle.
    XdpPping,

    /// Divide current RTT data into histograms and return the data for
    /// rendering.
    RttHistogram,

    /// Cound the number of mapped and unmapped hosts detected by the
    /// system.
    HostCounts,

    /// Retrieve a list of all unmapped IPs that have been detected
    /// carrying traffic.
    AllUnknownIps,

    /// Reload the `LibreQoS.py` program and return details of the
    /// reload run.
    ReloadLibreQoS,

    /// Retrieve raw queue data for a given circuit ID.
    GetRawQueueData(String), // The string is the circuit ID

    /// Requests a real-time adjustment of the `lqosd` tuning settings
    UpdateLqosDTuning(u64, Tunables),

    /// Requests that the configuration be updated
    UpdateLqosdConfig(Box<lqos_config::Config>),

    /// Request that we start watching a circuit's queue
    WatchQueue(String),

    /// Request that the Rust side of things validate the CSV
    ValidateShapedDevicesCsv,

    /// Request details of part of the network tree
    GetNetworkMap {
        /// The parent of the map to retrieve
        parent: usize,
    },

    /// Request the full network tree
    GetFullNetworkMap,

    /// Retrieves the top N queues from the root level, and summarizes
    /// the others as "other"
    TopMapQueues(usize),

    /// Retrieve node names from network.json
    GetNodeNamesFromIds(Vec<usize>),

    /// Get all circuits and usage statistics
    GetAllCircuits,

    /// Get circuit usage statistics for a single circuit ID
    GetCircuitById {
        /// Circuit ID to query
        circuit_id: String,
    },

    /// Retrieve stats for all queues above a named circuit id
    GetFunnel {
        /// Circuit being analyzed, as the named circuit id
        target: String,
    },

    /// Obtain the lqosd statistics
    GetLqosStats,

    /// Tell Heimdall to hyper-focus on an IP address for a bit
    GatherPacketData(String),

    /// Give me a dump of the last 10 seconds of packet headers
    GetPacketHeaderDump(usize),

    /// Give me a libpcap format packet dump (shortened) of the last 10 seconds
    GetPcapDump(usize),

    /// If running on Equinix (the `equinix_test` feature is enabled),
    /// display a "run bandwidht test" link.
    #[cfg(feature = "equinix_tests")]
    RequestLqosEquinixTest,

    /// Request a dump of all active flows. This can be a lot of data.
    /// so this is intended for debugging
    DumpActiveFlows,

    /// Count the nubmer of active flows.
    CountActiveFlows,

    /// Top Flows Reports
    TopFlows {
        /// The type of top report to request
        flow_type: TopFlowType,
        /// The number of flows to return
        n: u32,
    },

    /// Flows by IP Address
    FlowsByIp(String),

    /// Current Endpoints by Country
    CurrentEndpointsByCountry,

    /// Lat/Lon of Endpoints
    CurrentEndpointLatLon,

    /// Duration of flows
    FlowDuration,

    /// Ether Protocol Summary
    EtherProtocolSummary,

    /// IP Protocol Summary
    IpProtocolSummary,

    /// Submit a piece of information to the blackboard
    BlackboardData {
        /// The subsystem to which the data applies
        subsystem: BlackboardSystem,
        /// The key for the data
        key: String,
        /// The value for the data
        value: String,
    },

    /// Submit binary data to the blackboard
    BlackboardBlob {
        /// The subsystem to which the data applies
        tag: String,
        /// The part of the data being submitted
        part: usize,
        /// The binary data
        blob: Vec<u8>,
    },

    /// Finish a blackboard session
    BlackboardFinish,

    // lqos_bakery requests
    /// Start a bakery session
    BakeryStart,
    /// Request a bakery commit
    BakeryCommit,
    /// Setup the MQ top
    BakeryMqSetup {
        /// The number of queues available
        queues_available: usize,
        /// The "stick offset" calculated in LibreQoS.py
        stick_offset: usize,
    },
    /// Add a site to the bakery
    BakeryAddSite {
        /// The site hash, which is a unique identifier for the site
        site_hash: i64,
        /// The parent class ID for the site
        parent_class_id: TcHandle,
        /// The upload parent class ID for the site
        up_parent_class_id: TcHandle,
        /// The class minor version for the site
        class_minor: u16,
        /// The minimum download bandwidth for the site
        download_bandwidth_min: f32,
        /// The minimum upload bandwidth for the site
        upload_bandwidth_min: f32,
        /// The maximum download bandwidth for the site
        download_bandwidth_max: f32,
        /// The maximum upload bandwidth for the site
        upload_bandwidth_max: f32,
    },
    /// Add a circuit to the bakery
    BakeryAddCircuit {
        /// The circuit hash, which is a unique identifier for the circuit
        circuit_hash: i64,
        /// The parent class ID for the circuit
        parent_class_id: TcHandle,
        /// The upload parent class ID for the circuit
        up_parent_class_id: TcHandle,
        /// The class minor version for the circuit
        class_minor: u16,
        /// The minimum download bandwidth for the circuit
        download_bandwidth_min: f32,
        /// The minimum upload bandwidth for the circuit
        upload_bandwidth_min: f32,
        /// The maximum download bandwidth for the circuit
        download_bandwidth_max: f32,
        /// The maximum upload bandwidth for the circuit
        upload_bandwidth_max: f32,
        /// The class major version for the circuit
        class_major: u16,
        /// The upload class major version for the circuit
        up_class_major: u16,
        /// Concatenated list of IP addresses for the circuit
        ip_addresses: String,
        /// Optional per-circuit SQM override: "cake" or "fq_codel"
        sqm_override: Option<String>,
    },

    /// Get current Stormguard statistics
    GetStormguardStats,

    /// Get current Stormguard debug snapshot
    GetStormguardDebug,

    /// Get current Bakery statistics
    GetBakeryStats,

    /// Announce that the API is ready
    ApiReady,

    /// Announce that the chatbot is ready
    ChatbotReady,

    /// Announce that the scheduler is ready
    SchedulerReady,

    /// Announce a scheduler error
    SchedulerError(String),

    /// Write an informational message to the lqosd logs
    LogInfo(String),

    /// Check the scheduler status
    CheckSchedulerStatus,

    /// Bakery: Change Site Speed
    BakeryChangeSiteSpeedLive {
        /// The hash of the site to target
        site_hash: i64,
        /// Commit download bandwidth in Mbps
        download_bandwidth_min: f32,
        /// Commit upload bandwidth in Mbps
        upload_bandwidth_min: f32,
        /// Ceiling download bandwidth in Mbps
        download_bandwidth_max: f32,
        /// Ceiling upload bandwidth in Mbps
        upload_bandwidth_max: f32,
    },
    /// Submit an urgent issue for high-priority operator visibility
    SubmitUrgentIssue {
        /// Source of the issue
        source: UrgentSource,
        /// Severity of the issue
        severity: UrgentSeverity,
        /// Machine-readable code for the issue (e.g. TC_U16_OVERFLOW)
        code: String,
        /// Human-readable message for display
        message: String,
        /// Optional JSON context payload
        context: Option<String>,
        /// Optional key to deduplicate repeated submissions
        dedupe_key: Option<String>,
    },

    /// Retrieve current urgent issues
    GetUrgentIssues,

    /// Clear a specific urgent issue by ID
    ClearUrgentIssue(u64),

    /// Clear all urgent issues
    ClearAllUrgentIssues,

    /// Retrieve device counts (shaped + unknown)
    GetDeviceCounts,

    /// Retrieve circuit counts (active + configured)
    GetCircuitCount,

    /// Retrieve flow map points (lat/lon endpoints)
    GetFlowMap,

    /// Retrieve list of ASNs with recent flow data
    GetAsnList,

    /// Retrieve list of countries with recent flow data
    GetCountryList,

    /// Retrieve list of protocols with recent flow data
    GetProtocolList,

    /// Retrieve flow timeline entries for an ASN
    GetAsnFlowTimeline {
        /// ASN number to filter
        asn: u32,
    },

    /// Retrieve flow timeline entries for a country
    GetCountryFlowTimeline {
        /// Country ISO code to filter
        iso_code: String,
    },

    /// Retrieve flow timeline entries for a protocol
    GetProtocolFlowTimeline {
        /// Protocol name to filter
        protocol: String,
    },

    /// Retrieve scheduler details (diagnostics)
    GetSchedulerDetails,

    /// Retrieve queue marks/drops totals
    GetQueueStatsTotal,

    /// Retrieve per-circuit capacity utilization
    GetCircuitCapacity,

    /// Retrieve per-node capacity utilization
    GetTreeCapacity,

    /// Retrieve aggregate TCP retransmit summary
    GetRetransmitSummary,

    /// Retrieve two-level tree summary
    GetTreeSummaryL2,

    /// Search circuits/devices/sites by term
    Search {
        /// Search term
        term: String,
    },

    /// Retrieve current global warning list
    GetGlobalWarnings,

    /// Is Insight Enabled?
    CheckInsight,

    /// Retrieve current Insight license summary (licensed + optional max circuits).
    GetInsightLicenseSummary,
}

/// Defines the parts of the blackboard
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Copy, Allocative)]
pub enum BlackboardSystem {
    /// The system as a whole
    System,
    /// A specific site
    Site,
    /// A specific circuit
    Circuit,
    /// A specific device
    Device,
}

/// Defines the type of "top" flow being requested
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Copy, Allocative)]
pub enum TopFlowType {
    /// Top flows by current estimated bandwidth use
    RateEstimate,
    /// Top flows by total bytes transferred
    Bytes,
    /// Top flows by total packets transferred
    Packets,
    /// Top flows by total drops
    Drops,
    /// Top flows by round-trip time estimate
    RoundTripTime,
}
